//! The naive symmetric market maker — the baseline every sophisticated
//! quoter is measured against.
//!
//! Quote `mid ± half_spread`, fixed size; requote on a timer or when the mid
//! moves; go one-sided when inventory breaches its cap. No inventory-aware
//! skew, no volatility awareness — deliberately: this is the "quote around
//! mid" strawman that Avellaneda–Stoikov must beat under identical
//! frictions, seeds and fees for its complexity to have earned its keep.

use backtester::strategy::{OwnFill, Strategy, TradeApi};
use serde::{Deserialize, Serialize};
use sim_core::{EventRecord, Price, Qty, Side, SimDuration, SymbolId};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NaiveMmConfig {
    pub symbol_index: u16,
    /// Half-spread in ticks around the mid.
    pub half_spread_ticks: i64,
    pub size_lots: u64,
    /// Requote timer period.
    pub requote_ms: i64,
    /// Requote early when |mid − last quoted mid| ≥ this.
    pub requote_move_ticks: i64,
    /// One-sided quoting beyond this absolute inventory (lots).
    pub inventory_cap_lots: i64,
}

impl Default for NaiveMmConfig {
    fn default() -> Self {
        Self {
            symbol_index: 0,
            half_spread_ticks: 2,
            size_lots: 5,
            requote_ms: 250,
            requote_move_ticks: 2,
            inventory_cap_lots: 50,
        }
    }
}

const TIMER_REQUOTE: u64 = 1;

#[derive(Debug)]
pub struct NaiveMm {
    config: NaiveMmConfig,
    symbol: SymbolId,
    last_quoted_mid: Option<i64>,
}

impl NaiveMm {
    #[must_use]
    pub fn new(config: NaiveMmConfig) -> Self {
        Self {
            symbol: SymbolId::new(config.symbol_index),
            config,
            last_quoted_mid: None,
        }
    }

    fn requote(&mut self, api: &mut TradeApi<'_, '_>) {
        let Some(mid) = api.mid_ticks(self.symbol) else {
            return; // no two-sided book yet
        };
        api.cancel_all(self.symbol);
        let position = api.position(self.symbol);
        let bid_price = mid - self.config.half_spread_ticks;
        let ask_price = mid + self.config.half_spread_ticks;
        let quote_bid = position < self.config.inventory_cap_lots;
        let quote_ask = position > -self.config.inventory_cap_lots;
        if quote_bid && bid_price >= 1 {
            api.submit_limit(
                self.symbol,
                Side::Buy,
                Price::new(bid_price),
                Qty::new(self.config.size_lots),
            );
        }
        if quote_ask {
            api.submit_limit(
                self.symbol,
                Side::Sell,
                Price::new(ask_price),
                Qty::new(self.config.size_lots),
            );
        }
        self.last_quoted_mid = Some(mid);
    }
}

impl Strategy for NaiveMm {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>) {
        api.set_timer(
            SimDuration::from_millis(self.config.requote_ms),
            TIMER_REQUOTE,
        );
    }

    fn on_market_data(&mut self, _record: &EventRecord, api: &mut TradeApi<'_, '_>) {
        if let (Some(mid), Some(last)) = (api.mid_ticks(self.symbol), self.last_quoted_mid)
            && (mid - last).abs() >= self.config.requote_move_ticks
        {
            self.requote(api);
        } else if self.last_quoted_mid.is_none() {
            self.requote(api);
        }
    }

    fn on_own_fill(&mut self, _fill: &OwnFill, api: &mut TradeApi<'_, '_>) {
        self.requote(api);
    }

    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>) {
        if token == TIMER_REQUOTE {
            self.requote(api);
            api.set_timer(
                SimDuration::from_millis(self.config.requote_ms),
                TIMER_REQUOTE,
            );
        }
    }

    fn on_end(&mut self, api: &mut TradeApi<'_, '_>) {
        api.cancel_all(self.symbol);
    }
}
