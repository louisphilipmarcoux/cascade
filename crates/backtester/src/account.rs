//! Exchange-truth accounting: cash, position, cost basis — exact `i128`
//! arithmetic, folded from the authoritative event log.
//!
//! Design: `cash`, `fees` and the signed **open cost basis** are the stored,
//! exact quantities; `realized` and `unrealized` are *derived* from them:
//!
//! - `realized  = cash + fees + open_cost`
//! - `unrealized(mark) = position × mark − open_cost`
//! - `equity(mark) = cash + position × mark`
//!
//! which makes the accounting identity (P19)
//! `equity = realized + unrealized − fees` hold **exactly, always**, by
//! construction — the average-cost split of closed lots can shift dust
//! between realized and unrealized (integer division of the basis), but can
//! never leak cash.

use serde::{Deserialize, Serialize};
use sim_core::{ArithmeticError, Cash, Instrument, Price, Qty, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Account {
    /// Quote-currency cash at e8 (starts at 0: `PnL` accounting, not balance).
    pub cash: Cash,
    /// Signed base position in lots.
    pub position_lots: i64,
    /// Total fees paid (net of rebates), e8.
    pub fees: Cash,
    /// Signed cost basis of the open position, e8 (positive for longs,
    /// negative for shorts; zero when flat).
    open_cost_e8: i128,
    /// Fill statistics.
    pub fills: u64,
    pub maker_fills: u64,
    pub gross_traded: Cash,
}

impl Account {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one of our fills. `side` is *our* side of the trade.
    pub fn apply_fill(
        &mut self,
        instrument: &Instrument,
        side: Side,
        price: Price,
        qty: Qty,
        fee: Cash,
        is_maker: bool,
    ) -> Result<(), ArithmeticError> {
        let notional = instrument.notional(price, qty)?;
        let lots = i64::try_from(qty.lots()).map_err(|_| ArithmeticError::Overflow {
            op: "Account::apply_fill lots",
        })?;
        // Exact by construction: notional = ticks × lots × tick_lot_value.
        let per_lot_e8 = notional.e8() / i128::from(qty.lots());

        match side {
            Side::Buy => self.cash = self.cash.checked_sub(notional)?,
            Side::Sell => self.cash = self.cash.checked_add(notional)?,
        }
        self.cash = self.cash.checked_sub(fee)?;
        self.fees = self.fees.checked_add(fee)?;
        self.gross_traded = self.gross_traded.checked_add(notional)?;
        self.fills += 1;
        if is_maker {
            self.maker_fills += 1;
        }

        let signed_fill = match side {
            Side::Buy => i128::from(lots),
            Side::Sell => -i128::from(lots),
        };
        let old_pos = i128::from(self.position_lots);
        if old_pos == 0 || old_pos.signum() == signed_fill.signum() {
            // Opening/extending: basis grows by the signed cost.
            self.open_cost_e8 += per_lot_e8 * signed_fill;
        } else {
            let closed = old_pos.abs().min(signed_fill.abs());
            if closed == old_pos.abs() {
                // Fully closed (possibly crossing): remainder reopens at the
                // fill price; the old basis is fully released to realized.
                let remainder = signed_fill + old_pos; // signed leftover fill
                self.open_cost_e8 = per_lot_e8 * remainder;
            } else {
                // Partially closed: release a proportional basis share.
                // Integer division shifts ≤1e-8/lot of dust between realized
                // and unrealized; the equity identity is unaffected.
                let share = self.open_cost_e8 * closed / old_pos.abs();
                self.open_cost_e8 -= share;
            }
        }
        self.position_lots += signed_fill as i64; // ±lots, fits by construction
        Ok(())
    }

    /// Realized `PnL` (derived; exact identity component).
    #[must_use]
    pub fn realized(&self) -> Cash {
        Cash::from_e8(self.cash.e8() + self.fees.e8() + self.open_cost_e8)
    }

    /// Unrealized `PnL` at `mark` (quote per lot, e8; derived).
    #[must_use]
    pub fn unrealized(&self, mark_per_lot_e8: i128) -> Cash {
        Cash::from_e8(i128::from(self.position_lots) * mark_per_lot_e8 - self.open_cost_e8)
    }

    /// Equity at `mark`: cash + position value.
    #[must_use]
    pub fn equity(&self, mark_per_lot_e8: i128) -> Cash {
        Cash::from_e8(self.cash.e8() + i128::from(self.position_lots) * mark_per_lot_e8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::SymbolId;

    fn inst() -> Instrument {
        // tick 0.01 → 1_000_000 e8; lot 1.0 → 100_000_000 e8.
        Instrument::new("T", SymbolId::new(0), 1_000_000, 100_000_000).unwrap()
    }

    #[test]
    fn round_trip_realizes_spread() {
        let inst = inst();
        let mut acct = Account::new();
        acct.apply_fill(
            &inst,
            Side::Buy,
            Price::new(100),
            Qty::new(2),
            Cash::ZERO,
            true,
        )
        .unwrap();
        assert_eq!(acct.position_lots, 2);
        acct.apply_fill(
            &inst,
            Side::Sell,
            Price::new(110),
            Qty::new(2),
            Cash::ZERO,
            false,
        )
        .unwrap();
        assert_eq!(acct.position_lots, 0);
        assert_eq!(acct.realized().e8(), 20_000_000);
        assert_eq!(acct.cash.e8(), 20_000_000);
        assert_eq!(acct.equity(0).e8(), 20_000_000);
    }

    #[test]
    fn crossing_zero_reopens_at_fill_price() {
        let inst = inst();
        let mut acct = Account::new();
        acct.apply_fill(
            &inst,
            Side::Buy,
            Price::new(100),
            Qty::new(1),
            Cash::ZERO,
            true,
        )
        .unwrap();
        acct.apply_fill(
            &inst,
            Side::Sell,
            Price::new(120),
            Qty::new(3),
            Cash::ZERO,
            true,
        )
        .unwrap();
        assert_eq!(acct.position_lots, -2);
        assert_eq!(acct.realized().e8(), 20_000_000, "closed 1 lot at +0.20");
        assert_eq!(acct.unrealized(120 * 1_000_000).e8(), 0);
        assert_eq!(acct.unrealized(110 * 1_000_000).e8(), 20_000_000);
    }

    /// P19 in its exact form, including a partial close whose basis split is
    /// not integer-divisible.
    #[test]
    fn identity_holds_exactly_even_with_dusty_splits() {
        let inst = inst();
        let mut acct = Account::new();
        let fee = Cash::from_e8(1_000);
        acct.apply_fill(&inst, Side::Buy, Price::new(100), Qty::new(1), fee, false)
            .unwrap();
        acct.apply_fill(&inst, Side::Buy, Price::new(101), Qty::new(2), fee, true)
            .unwrap();
        // Close 2 of 3: basis share = open_cost*2/3 — not divisible.
        acct.apply_fill(&inst, Side::Sell, Price::new(99), Qty::new(2), fee, false)
            .unwrap();
        for mark_ticks in [95i128, 100, 105] {
            let mark = mark_ticks * 1_000_000;
            let lhs = acct.equity(mark).e8();
            let rhs = acct.realized().e8() + acct.unrealized(mark).e8() - acct.fees.e8();
            assert_eq!(lhs, rhs, "identity must be exact at mark {mark_ticks}");
        }
        assert_eq!(acct.position_lots, 1);
    }
}
