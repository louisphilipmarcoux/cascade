//! Cointegrated-pair synthetic flow: two instruments whose mids share a
//! common random-walk factor plus an OU spread — a ground-truth generator
//! for the OU pairs strategy that needs no real data.
//!
//! `ln P_a = F + s/2`, `ln P_b = F − s/(2h)` where `F` is a common random
//! walk (the market factor, non-stationary) and `s` is a mean-reverting OU
//! spread. By construction `ln P_a − h·ln P_b = s·(1 + 1/2 − h/(2h)) …` is
//! stationary, so the pair is genuinely cointegrated with hedge ratio `h`.
//! Each instrument is driven to its target mid by a marketable-order flow
//! plus a maintained frame, exactly as the replay source frames a tape.

use serde::{Deserialize, Serialize};
use sim_core::{
    OrderKind, OwnerId, Price, Qty, Request, SimDuration, SimTime, SymbolId, TimeInForce,
};

use super::{FlowCtx, OrderFlowSource, OwnOrders, submit};
use crate::latency::standard_normal;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CointPairConfig {
    pub symbol_a: u16,
    pub symbol_b: u16,
    /// Hedge ratio h relating the two legs.
    pub hedge_ratio: f64,
    /// Update cadence (ms) at which both mids are re-driven.
    pub step_ms: i64,
    /// Common-factor log-return volatility per step.
    pub factor_vol: f64,
    /// OU spread: mean-reversion speed per step (`0 < theta_step < 1`).
    pub spread_theta: f64,
    /// OU spread: innovation volatility per step.
    pub spread_vol: f64,
    /// Initial log-price of the common factor.
    pub initial_log_price: f64,
    /// Frame half-spread (ticks) and depth (lots) per leg.
    pub frame_spread_ticks: i64,
    pub frame_lots: u64,
    /// Taker size (lots) that pushes each mid toward target.
    pub taker_lots: u64,
}

impl Default for CointPairConfig {
    fn default() -> Self {
        Self {
            symbol_a: 0,
            symbol_b: 1,
            hedge_ratio: 1.0,
            step_ms: 100,
            factor_vol: 0.0005,
            spread_theta: 0.02,
            spread_vol: 0.01,
            initial_log_price: 9.21, // ~10000
            frame_spread_ticks: 3,
            frame_lots: 1_000,
            taker_lots: 50,
        }
    }
}

/// One leg's live frame (bid id/price, ask id/price).
#[derive(Debug, Default)]
struct Frame {
    bid: Option<(sim_core::OrderId, i64)>,
    ask: Option<(sim_core::OrderId, i64)>,
    last_target: Option<i64>,
}

#[derive(Debug)]
pub struct CointPairSource {
    config: CointPairConfig,
    owner: OwnerId,
    factor: f64,
    spread: f64,
    next_step: Option<SimTime>,
    /// Virtual time of the step whose actions are currently queued — all of
    /// them fire at this instant (distinct from `next_step`, the following
    /// step's time).
    current_step: SimTime,
    /// Pending actions for the current step (drained before advancing).
    queue: std::collections::VecDeque<Request>,
    frame_a: Frame,
    frame_b: Frame,
    own: OwnOrders,
    /// Tick scale: ln-price → ticks. Both legs use 1 tick = 0.01 (e-2) here.
    tick_ln_scale: f64,
    started: bool,
}

impl CointPairSource {
    #[must_use]
    pub fn new(config: CointPairConfig, owner: OwnerId) -> Self {
        Self {
            factor: config.initial_log_price,
            spread: 0.0,
            owner,
            config,
            next_step: None,
            current_step: SimTime::EPOCH,
            queue: std::collections::VecDeque::new(),
            frame_a: Frame::default(),
            frame_b: Frame::default(),
            own: OwnOrders::default(),
            // price = exp(ln_price); ticks = price / 0.01 = price * 100.
            tick_ln_scale: 100.0,
            started: false,
        }
    }

    fn target_ticks(&self, log_price: f64) -> i64 {
        (libm::exp(log_price) * self.tick_ln_scale).round() as i64
    }

    /// Enqueue the frame refresh + a mid-driving taker for one leg.
    fn drive_leg(
        &mut self,
        id_gen: &mut sim_core::IdGen,
        symbol: SymbolId,
        target: i64,
        is_a: bool,
    ) {
        let frame = if is_a {
            &mut self.frame_a
        } else {
            &mut self.frame_b
        };
        let moved = frame.last_target != Some(target);
        if moved {
            if let Some((oid, _)) = frame.bid.take() {
                self.queue.push_back(Request::Cancel { id: oid });
            }
            if let Some((oid, _)) = frame.ask.take() {
                self.queue.push_back(Request::Cancel { id: oid });
            }
            let bid_px = (target - self.config.frame_spread_ticks).max(1);
            let ask_px = target + self.config.frame_spread_ticks;
            let (bid_id, bid_req) = submit(
                id_gen,
                self.owner,
                symbol,
                sim_core::Side::Buy,
                OrderKind::Limit(Price::new(bid_px)),
                Qty::new(self.config.frame_lots),
                TimeInForce::Gtc,
            );
            let (ask_id, ask_req) = submit(
                id_gen,
                self.owner,
                symbol,
                sim_core::Side::Sell,
                OrderKind::Limit(Price::new(ask_px)),
                Qty::new(self.config.frame_lots),
                TimeInForce::Gtc,
            );
            self.queue.push_back(bid_req);
            self.queue.push_back(ask_req);
            let frame = if is_a {
                &mut self.frame_a
            } else {
                &mut self.frame_b
            };
            frame.bid = Some((bid_id, bid_px));
            frame.ask = Some((ask_id, ask_px));
            frame.last_target = Some(target);
        }
    }

    fn step(&mut self, ctx: &mut FlowCtx<'_>) {
        // Advance the latent processes deterministically.
        self.factor += self.config.factor_vol * standard_normal(ctx.rng);
        self.spread = (1.0 - self.config.spread_theta) * self.spread
            + self.config.spread_vol * standard_normal(ctx.rng);

        let log_a = self.factor + self.spread / 2.0;
        let log_b = self.factor - self.spread / (2.0 * self.config.hedge_ratio);
        let target_a = self.target_ticks(log_a);
        let target_b = self.target_ticks(log_b);

        let sym_a = SymbolId::new(self.config.symbol_a);
        let sym_b = SymbolId::new(self.config.symbol_b);
        self.drive_leg(ctx.id_gen, sym_a, target_a, true);
        self.drive_leg(ctx.id_gen, sym_b, target_b, false);
    }
}

impl OrderFlowSource for CointPairSource {
    fn next(&mut self, ctx: &mut FlowCtx<'_>) -> Option<(SimTime, Request)> {
        if !self.started {
            self.started = true;
            self.next_step = Some(ctx.now);
        }
        loop {
            // All queued actions belong to `current_step` and fire together.
            if let Some(request) = self.queue.pop_front() {
                return Some((self.current_step, request));
            }
            let at = self.next_step?;
            let step_at = at.max(ctx.now);
            self.current_step = step_at;
            self.step(ctx);
            self.next_step = step_at
                .checked_add(SimDuration::from_millis(self.config.step_ms))
                .ok();
            if self.queue.is_empty() {
                // No frame move this step; keep stepping.
                continue;
            }
            let request = self.queue.pop_front().expect("non-empty");
            return Some((step_at, request));
        }
    }

    fn on_own_record(&mut self, record: &sim_core::EventRecord) {
        self.own.on_record(record);
    }
}
