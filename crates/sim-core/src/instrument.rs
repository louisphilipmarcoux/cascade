//! Instrument metadata and the single decimal ↔ fixed-point boundary.
//!
//! An [`Instrument`] carries the tick size (quote currency per tick) and lot
//! size (base asset per lot) at e8 scale. Construction validates that
//! `tick_size_e8 × lot_size_e8` is divisible by 1e8, which makes the derived
//! per-(tick·lot) value exact — and therefore makes **every** notional, fee
//! and `PnL` computation in the system exact integer arithmetic.

use serde::{Deserialize, Serialize};

use crate::error::{ArithmeticError, CoreError};
use crate::ids::SymbolId;
use crate::price::{Cash, E8, Price, Qty};

/// Tradeable instrument metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    /// Human-readable symbol, e.g. `"BTCUSDT"`.
    pub name: String,
    pub symbol: SymbolId,
    /// Quote currency per tick, at e8 scale (e.g. tick size 0.10 → `10_000_000`).
    pub tick_size_e8: i64,
    /// Base asset per lot, at e8 scale (e.g. lot size 0.001 → `100_000`).
    pub lot_size_e8: i64,
    /// Quote currency per (tick × lot), at e8 scale. Derived; exact by
    /// construction.
    tick_lot_value_e8: i64,
}

impl Instrument {
    pub fn new(
        name: impl Into<String>,
        symbol: SymbolId,
        tick_size_e8: i64,
        lot_size_e8: i64,
    ) -> Result<Self, CoreError> {
        let name = name.into();
        if tick_size_e8 <= 0 || lot_size_e8 <= 0 {
            return Err(CoreError::InvalidInstrument {
                symbol: name,
                reason: "tick_size and lot_size must be positive".into(),
            });
        }
        let product = i128::from(tick_size_e8) * i128::from(lot_size_e8);
        if product % E8 != 0 {
            return Err(CoreError::InvalidInstrument {
                symbol: name,
                reason: format!(
                    "tick_size_e8 ({tick_size_e8}) × lot_size_e8 ({lot_size_e8}) is not a \
                     multiple of 1e8; notional arithmetic would be inexact"
                ),
            });
        }
        let value = product / E8;
        let tick_lot_value_e8 = i64::try_from(value).map_err(|_| CoreError::InvalidInstrument {
            symbol: name.clone(),
            reason: "tick·lot value overflows i64".into(),
        })?;
        Ok(Self {
            name,
            symbol,
            tick_size_e8,
            lot_size_e8,
            tick_lot_value_e8,
        })
    }

    /// Quote currency per (tick × lot) at e8 scale.
    #[must_use]
    pub const fn tick_lot_value_e8(&self) -> i64 {
        self.tick_lot_value_e8
    }

    /// Exact notional value of `qty` at `price`.
    pub fn notional(&self, price: Price, qty: Qty) -> Result<Cash, ArithmeticError> {
        let ticks = i128::from(price.ticks());
        let lots = i128::from(qty.lots());
        ticks
            .checked_mul(lots)
            .and_then(|tl| tl.checked_mul(i128::from(self.tick_lot_value_e8)))
            .map(Cash::from_e8)
            .ok_or(ArithmeticError::Overflow {
                op: "Instrument::notional",
            })
    }

    /// Convert an e8-scaled decimal price to ticks, requiring exact
    /// divisibility (no silent rounding at the data boundary).
    #[must_use]
    pub fn price_from_e8_exact(&self, price_e8: i64) -> Option<Price> {
        (price_e8 % self.tick_size_e8 == 0).then(|| Price::new(price_e8 / self.tick_size_e8))
    }

    /// Convert an e8-scaled decimal quantity to lots, requiring exact
    /// divisibility.
    #[must_use]
    pub fn qty_from_e8_exact(&self, qty_e8: i64) -> Option<Qty> {
        (qty_e8 >= 0 && qty_e8 % self.lot_size_e8 == 0)
            .then(|| Qty::new((qty_e8 / self.lot_size_e8) as u64))
    }

    /// Ticks → e8-scaled decimal price.
    pub fn price_to_e8(&self, price: Price) -> Result<i64, ArithmeticError> {
        price
            .ticks()
            .checked_mul(self.tick_size_e8)
            .ok_or(ArithmeticError::Overflow {
                op: "Instrument::price_to_e8",
            })
    }

    /// Lots → e8-scaled decimal quantity.
    pub fn qty_to_e8(&self, qty: Qty) -> Result<i64, ArithmeticError> {
        i64::try_from(qty.lots())
            .ok()
            .and_then(|lots| lots.checked_mul(self.lot_size_e8))
            .ok_or(ArithmeticError::Overflow {
                op: "Instrument::qty_to_e8",
            })
    }
}

/// Parse a decimal string (e.g. `"0.10"`, `"-3.5"`, `"12"`) into an exact
/// e8-scaled integer. Rejects more than 8 fractional digits (unless they are
/// trailing zeros) — the data boundary refuses precision it cannot represent.
pub fn parse_decimal_e8(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() {
        return Err(format!("empty decimal: {s:?}"));
    }
    let (int_part, frac_part) = match digits.split_once('.') {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(format!("empty decimal: {s:?}"));
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid decimal: {s:?}"));
    }
    let mut frac = frac_part.to_string();
    while frac.len() > 8 {
        if frac.ends_with('0') {
            frac.pop();
        } else {
            return Err(format!(
                "decimal {s:?} has more than 8 fractional digits; not representable at e8 scale"
            ));
        }
    }
    while frac.len() < 8 {
        frac.push('0');
    }
    let int_val: i64 = if int_part.is_empty() {
        0
    } else {
        int_part
            .parse()
            .map_err(|_| format!("integer part out of range: {s:?}"))?
    };
    let frac_val: i64 = frac
        .parse()
        .map_err(|_| format!("invalid decimal: {s:?}"))?;
    let magnitude = int_val
        .checked_mul(100_000_000)
        .and_then(|v| v.checked_add(frac_val))
        .ok_or_else(|| format!("decimal out of range: {s:?}"))?;
    Ok(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn btcusdt() -> Instrument {
        // tick 0.10 quote, lot 0.001 base → tick·lot value 0.0001 quote (10_000 e8).
        Instrument::new("BTCUSDT", SymbolId::new(0), 10_000_000, 100_000).unwrap()
    }

    #[test]
    fn tick_lot_value_is_exact() {
        assert_eq!(btcusdt().tick_lot_value_e8(), 10_000);
    }

    #[test]
    fn rejects_inexact_tick_lot_product() {
        // tick 0.000001 (100 e8) × lot 0.001 (100_000 e8) = 1e7, not divisible by 1e8.
        let err = Instrument::new("BAD", SymbolId::new(1), 100, 100_000);
        assert!(err.is_err());
    }

    #[test]
    fn notional_is_exact_integer_math() {
        let inst = btcusdt();
        // price 60_000.00 → 600_000 ticks; qty 0.005 → 5 lots.
        let price = inst
            .price_from_e8_exact(parse_decimal_e8("60000").unwrap())
            .unwrap();
        let qty = inst
            .qty_from_e8_exact(parse_decimal_e8("0.005").unwrap())
            .unwrap();
        // notional = 60000 × 0.005 = 300 quote = 300e8.
        assert_eq!(inst.notional(price, qty).unwrap(), Cash::from_e8(300 * E8));
    }

    #[test]
    fn boundary_conversion_requires_exactness() {
        let inst = btcusdt();
        assert!(
            inst.price_from_e8_exact(parse_decimal_e8("60000.05").unwrap())
                .is_none()
        );
        assert!(
            inst.qty_from_e8_exact(parse_decimal_e8("0.0005").unwrap())
                .is_none()
        );
    }

    #[test]
    fn parse_decimal_e8_cases() {
        assert_eq!(parse_decimal_e8("0.10").unwrap(), 10_000_000);
        assert_eq!(parse_decimal_e8("12").unwrap(), 1_200_000_000);
        assert_eq!(parse_decimal_e8("-3.5").unwrap(), -350_000_000);
        assert_eq!(parse_decimal_e8("0.00000001").unwrap(), 1);
        assert_eq!(parse_decimal_e8("1.000000010").unwrap(), 100_000_001);
        assert!(parse_decimal_e8("0.000000001").is_err());
        assert!(parse_decimal_e8("abc").is_err());
        assert!(parse_decimal_e8("").is_err());
        assert!(parse_decimal_e8(".").is_err());
    }
}
