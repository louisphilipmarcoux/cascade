//! Ornstein–Uhlenbeck pairs trading.
//!
//! The spread `X_t = ln P_a − h·ln P_b` of a cointegrated pair is modelled as
//! a mean-reverting OU process; entry/exit thresholds are derived from the
//! *fitted* stationary distribution (z-scores), not ad-hoc levels. The hedge
//! ratio `h` and the pair come from the research layer / scenario (Python
//! Engle–Granger); this strategy fits the OU parameters online with a
//! rolling window and refits on a walk-forward cadence.
//!
//! Trades are expressed as market orders on both legs (dollar-neutral by the
//! hedge ratio, rounded to lots), so `PnL` is realized through the real
//! matching engine under the same frictions as everything else.

use backtester::strategy::{OwnFill, Strategy, TradeApi};
use serde::{Deserialize, Serialize};
use sim_core::{EventRecord, OrderKind, Qty, Side, SimDuration, SymbolId, TimeInForce};

use crate::estimators::{OuFit, fit_ou};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OuPairsConfig {
    pub symbol_a: u16,
    pub symbol_b: u16,
    /// Hedge ratio h (from research-layer cointegration).
    pub hedge_ratio: f64,
    /// Spread sampling cadence (ms).
    pub sample_ms: i64,
    /// Rolling window length (samples) for the OU fit.
    pub window: usize,
    /// Refit cadence (samples) — walk-forward.
    pub refit_every: usize,
    /// Entry when |z| ≥ this.
    pub z_entry: f64,
    /// Exit when |z| ≤ this.
    pub z_exit: f64,
    /// Hard stop when |z| ≥ this (fit likely broke).
    pub z_stop: f64,
    /// Leg-a size in lots per unit position.
    pub size_a_lots: u64,
}

impl Default for OuPairsConfig {
    fn default() -> Self {
        Self {
            symbol_a: 0,
            symbol_b: 1,
            hedge_ratio: 1.0,
            sample_ms: 1000,
            window: 600,
            refit_every: 120,
            z_entry: 2.0,
            z_exit: 0.5,
            z_stop: 4.0,
            size_a_lots: 10,
        }
    }
}

const TIMER_SAMPLE: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairPosition {
    Flat,
    /// Long spread: long A, short B (entered when z very negative).
    LongSpread,
    /// Short spread: short A, long B (entered when z very positive).
    ShortSpread,
}

#[derive(Debug)]
pub struct OuPairs {
    config: OuPairsConfig,
    sym_a: SymbolId,
    sym_b: SymbolId,
    spread_window: std::collections::VecDeque<f64>,
    samples_since_fit: usize,
    fit: Option<OuFit>,
    position: PairPosition,
}

impl OuPairs {
    #[must_use]
    pub fn new(config: OuPairsConfig) -> Self {
        Self {
            sym_a: SymbolId::new(config.symbol_a),
            sym_b: SymbolId::new(config.symbol_b),
            config,
            spread_window: std::collections::VecDeque::new(),
            samples_since_fit: 0,
            fit: None,
            position: PairPosition::Flat,
        }
    }

    /// Current spread `ln P_a − h·ln P_b` from delayed mids, or `None` if
    /// either leg lacks a two-sided book.
    fn spread(&self, api: &TradeApi<'_, '_>) -> Option<f64> {
        let mid_a = api.mid_ticks(self.sym_a)? as f64;
        let mid_b = api.mid_ticks(self.sym_b)? as f64;
        if mid_a <= 0.0 || mid_b <= 0.0 {
            return None;
        }
        Some(libm::log(mid_a) - self.config.hedge_ratio * libm::log(mid_b))
    }

    fn trade_to(&mut self, target: PairPosition, api: &mut TradeApi<'_, '_>) {
        if target == self.position {
            return;
        }
        // Compute the required delta in "spread units" and translate to leg
        // orders. One spread unit = size_a_lots on A, h·size_a_lots on B.
        let current = match self.position {
            PairPosition::Flat => 0i64,
            PairPosition::LongSpread => 1,
            PairPosition::ShortSpread => -1,
        };
        let goal = match target {
            PairPosition::Flat => 0i64,
            PairPosition::LongSpread => 1,
            PairPosition::ShortSpread => -1,
        };
        let delta = goal - current; // −2..=2 spread units
        if delta == 0 {
            return;
        }
        // Long spread ⇒ buy A, sell B.
        let (a_side, b_side) = if delta > 0 {
            (Side::Buy, Side::Sell)
        } else {
            (Side::Sell, Side::Buy)
        };
        let units = delta.unsigned_abs();
        let a_lots = self.config.size_a_lots * units;
        let b_lots = ((self.config.hedge_ratio * self.config.size_a_lots as f64).round() as u64)
            .max(1)
            * units;
        api.submit(
            self.sym_a,
            a_side,
            OrderKind::Market,
            Qty::new(a_lots),
            TimeInForce::Ioc,
        );
        api.submit(
            self.sym_b,
            b_side,
            OrderKind::Market,
            Qty::new(b_lots),
            TimeInForce::Ioc,
        );
        self.position = target;
    }

    fn on_sample(&mut self, api: &mut TradeApi<'_, '_>) {
        let Some(spread) = self.spread(api) else {
            return;
        };
        self.spread_window.push_back(spread);
        while self.spread_window.len() > self.config.window {
            self.spread_window.pop_front();
        }
        self.samples_since_fit += 1;

        // Walk-forward refit.
        if self.fit.is_none() || self.samples_since_fit >= self.config.refit_every {
            if self.spread_window.len() >= self.config.window.min(30) {
                let series: Vec<f64> = self.spread_window.iter().copied().collect();
                let dt = self.config.sample_ms as f64 / 1000.0;
                self.fit = fit_ou(&series, dt).ok();
            }
            self.samples_since_fit = 0;
        }

        let Some(fit) = self.fit else { return };
        let z = fit.z_score(spread);

        match self.position {
            PairPosition::Flat => {
                if z >= self.config.z_entry {
                    self.trade_to(PairPosition::ShortSpread, api); // spread rich → short it
                } else if z <= -self.config.z_entry {
                    self.trade_to(PairPosition::LongSpread, api); // spread cheap → long it
                }
            }
            PairPosition::LongSpread | PairPosition::ShortSpread => {
                if z.abs() <= self.config.z_exit || z.abs() >= self.config.z_stop {
                    self.trade_to(PairPosition::Flat, api);
                }
            }
        }
    }
}

impl Strategy for OuPairs {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>) {
        api.set_timer(
            SimDuration::from_millis(self.config.sample_ms),
            TIMER_SAMPLE,
        );
    }

    fn on_market_data(&mut self, _record: &EventRecord, _api: &mut TradeApi<'_, '_>) {}

    fn on_own_fill(&mut self, _fill: &OwnFill, _api: &mut TradeApi<'_, '_>) {}

    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>) {
        if token == TIMER_SAMPLE {
            self.on_sample(api);
            api.set_timer(
                SimDuration::from_millis(self.config.sample_ms),
                TIMER_SAMPLE,
            );
        }
    }

    fn on_end(&mut self, api: &mut TradeApi<'_, '_>) {
        // Unwind at the end so PnL is inventory-neutral.
        self.trade_to(PairPosition::Flat, api);
    }
}
