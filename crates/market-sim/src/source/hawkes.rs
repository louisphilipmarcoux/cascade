//! Hawkes-driven order flow: the self-exciting upgrade over Poisson.
//!
//! Four dimensions — market buy/sell and limit buy/sell — driven by one
//! multivariate exponential-kernel [`HawkesProcess`]; placement and sizing
//! come from the shared [`PlacementModel`] so the *only* difference from the
//! Poisson source is the point process. Cancels remain an independent
//! Poisson clock (a documented modeling choice, revisited with the
//! stylized-facts results).

use serde::{Deserialize, Serialize};
use sim_core::{
    OrderKind, OwnerId, Price, Qty, Request, SimDuration, SimTime, SymbolId, TimeInForce,
};

use super::{BookView, FlowCtx, OrderFlowSource, OwnOrders, PlacementModel, next_arrival, submit};
use crate::hawkes::HawkesProcess;

use rand::Rng as _;

/// Which flow class each Hawkes dimension drives.
pub const DIMS: [&str; 4] = ["market_buy", "market_sell", "limit_buy", "limit_sell"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HawkesFlowConfig {
    /// Baselines per dim (events/sec), order per [`DIMS`].
    pub mu: Vec<f64>,
    /// Excitation matrix α[m][n] (1/sec).
    pub alpha: Vec<Vec<f64>>,
    /// Shared decay (1/sec).
    pub beta: f64,
    /// Independent cancel rate of own resting orders (events/sec).
    pub cancel_rate: f64,
    pub initial_mid_ticks: i64,
    pub placement: PlacementModel,
}

impl Default for HawkesFlowConfig {
    fn default() -> Self {
        // Mildly self- and cross-exciting flow, branching ratio ≈ 0.55.
        Self {
            mu: vec![1.0, 1.0, 14.0, 14.0],
            alpha: vec![
                vec![0.9, 0.3, 0.05, 0.05],
                vec![0.3, 0.9, 0.05, 0.05],
                vec![0.6, 0.2, 0.9, 0.3],
                vec![0.2, 0.6, 0.3, 0.9],
            ],
            beta: 4.0,
            cancel_rate: 9.0,
            initial_mid_ticks: 10_000,
            placement: PlacementModel::default(),
        }
    }
}

#[derive(Debug)]
pub struct HawkesSource {
    config: HawkesFlowConfig,
    process: HawkesProcess,
    symbol: SymbolId,
    owner: OwnerId,
    own: OwnOrders,
    next_cancel: Option<SimTime>,
    /// Next Hawkes event, pre-drawn (peeked) so cancels can interleave.
    pending: Option<(SimTime, usize)>,
    started: bool,
}

impl HawkesSource {
    pub fn new(config: HawkesFlowConfig, symbol: SymbolId, owner: OwnerId) -> Result<Self, String> {
        let process = HawkesProcess::new(config.mu.clone(), config.alpha.clone(), config.beta)?;
        if process.dims() != 4 {
            return Err("hawkes flow requires exactly 4 dims (see DIMS)".into());
        }
        Ok(Self {
            config,
            process,
            symbol,
            owner,
            own: OwnOrders::default(),
            next_cancel: None,
            pending: None,
            started: false,
        })
    }

    fn reference_ticks(&self, book: &dyn BookView) -> i64 {
        match (book.best_bid(), book.best_ask()) {
            (Some(b), Some(a)) => i64::midpoint(b.ticks(), a.ticks()),
            (Some(b), None) => b.ticks(),
            (None, Some(a)) => a.ticks(),
            (None, None) => self.config.initial_mid_ticks,
        }
    }

    fn draw_next(&mut self, rng: &mut sim_core::rng::Rng) {
        self.pending = self.process.next_event(rng).map(|(seconds, dim)| {
            let nanos = (seconds * 1e9).min(u64::MAX as f64) as u64;
            (SimTime::from_nanos(nanos), dim)
        });
    }
}

impl OrderFlowSource for HawkesSource {
    fn next(&mut self, ctx: &mut FlowCtx<'_>) -> Option<(SimTime, Request)> {
        if !self.started {
            self.started = true;
            self.next_cancel = Some(next_arrival(ctx.now, self.config.cancel_rate, ctx.rng));
            self.draw_next(ctx.rng);
        }
        loop {
            let hawkes_at = self.pending.map(|(t, _)| t);
            let cancel_at = self.next_cancel;
            // Earliest of the two clocks; Hawkes wins ties.
            let use_cancel = match (hawkes_at, cancel_at) {
                (None, None) => return None,
                (Some(_), None) => false,
                (None, Some(_)) => true,
                (Some(h), Some(c)) => c < h,
            };
            if use_cancel {
                let at = cancel_at.expect("checked");
                self.next_cancel = Some(next_arrival(at, self.config.cancel_rate, ctx.rng));
                let pick = ctx.rng.random::<u64>() as usize;
                if let Some(id) = self.own.pick(pick) {
                    return Some((at, Request::Cancel { id }));
                }
                continue; // nothing resting; consume the arrival
            }
            let (at, dim) = self.pending.take().expect("checked");
            self.draw_next(ctx.rng);
            let side = if dim % 2 == 0 {
                sim_core::Side::Buy
            } else {
                sim_core::Side::Sell
            };
            let qty = Qty::new(self.config.placement.size_lots(ctx.rng));
            let request = if dim < 2 {
                // Market order (IOC).
                submit(
                    ctx.id_gen,
                    self.owner,
                    self.symbol,
                    side,
                    OrderKind::Market,
                    qty,
                    TimeInForce::Ioc,
                )
                .1
            } else {
                let reference = self.reference_ticks(ctx.book);
                let depth = self.config.placement.depth_ticks(ctx.rng);
                let price = match side {
                    sim_core::Side::Buy => reference - depth,
                    sim_core::Side::Sell => reference + depth,
                };
                if price < 1 {
                    continue;
                }
                submit(
                    ctx.id_gen,
                    self.owner,
                    self.symbol,
                    side,
                    OrderKind::Limit(Price::new(price)),
                    qty,
                    TimeInForce::Gtc,
                )
                .1
            };
            // Never schedule before now (first event may predate warmup).
            let at = if at < ctx.now {
                ctx.now
                    .checked_add(SimDuration::from_nanos(1))
                    .unwrap_or(ctx.now)
            } else {
                at
            };
            return Some((at, request));
        }
    }

    fn on_own_record(&mut self, record: &sim_core::EventRecord) {
        self.own.on_record(record);
    }
}
