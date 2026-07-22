//! TWAP execution: equal slices over the horizon — the baseline Almgren–
//! Chriss is measured against.

use backtester::strategy::{OwnFill, Strategy, TradeApi};
use serde::{Deserialize, Serialize};
use sim_core::{EventRecord, OrderKind, Qty, Side, SimDuration, SymbolId, TimeInForce};

use super::ExecutionConfig;

// `flatten` is incompatible with `deny_unknown_fields`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TwapParams {
    #[serde(flatten)]
    pub exec: ExecutionConfig,
}

/// Equal slices summing exactly to the target (drift on the last slice).
#[must_use]
pub fn schedule(exec: &ExecutionConfig) -> Vec<i64> {
    let n = i64::from(exec.slices.max(1));
    let total = exec.target_lots;
    let per = total / n;
    let mut trades = vec![per; n as usize];
    if let Some(last) = trades.last_mut() {
        *last += total - per * n;
    }
    trades
}

const TIMER_SLICE: u64 = 1;

#[derive(Debug)]
pub struct Twap {
    params: TwapParams,
    symbol: SymbolId,
    trades: Vec<i64>,
    next_slice: usize,
    slice_dt_ms: i64,
}

impl Twap {
    #[must_use]
    pub fn new(params: TwapParams) -> Self {
        let trades = schedule(&params.exec);
        let slice_dt_ms =
            ((params.exec.horizon_secs / f64::from(params.exec.slices.max(1))) * 1000.0) as i64;
        Self {
            symbol: SymbolId::new(params.exec.symbol_index),
            params,
            trades,
            next_slice: 0,
            slice_dt_ms: slice_dt_ms.max(1),
        }
    }

    fn fire_slice(&mut self, api: &mut TradeApi<'_, '_>) {
        if self.next_slice >= self.trades.len() {
            return;
        }
        let lots = self.trades[self.next_slice];
        self.next_slice += 1;
        if lots == 0 {
            return;
        }
        let side = if self.params.exec.target_lots >= 0 {
            Side::Sell
        } else {
            Side::Buy
        };
        api.submit(
            self.symbol,
            side,
            OrderKind::Market,
            Qty::new(lots.unsigned_abs()),
            TimeInForce::Ioc,
        );
    }
}

impl Strategy for Twap {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>) {
        let warmup_ms = (self.params.exec.warmup_secs * 1000.0) as i64;
        api.set_timer(SimDuration::from_millis(warmup_ms.max(0)), TIMER_SLICE);
    }

    fn on_market_data(&mut self, _record: &EventRecord, _api: &mut TradeApi<'_, '_>) {}

    fn on_own_fill(&mut self, _fill: &OwnFill, _api: &mut TradeApi<'_, '_>) {}

    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>) {
        if token == TIMER_SLICE && self.next_slice < self.trades.len() {
            self.fire_slice(api);
            api.set_timer(SimDuration::from_millis(self.slice_dt_ms), TIMER_SLICE);
        }
    }

    fn on_end(&mut self, _api: &mut TradeApi<'_, '_>) {}
}
