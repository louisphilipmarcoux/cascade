//! Poisson (zero-intelligence) order flow — the baseline the Hawkes source
//! is measured against.
//!
//! Independent exponential inter-arrivals per event class (limit orders,
//! market orders, cancels of own resting orders), placement via the shared
//! [`PlacementModel`]. Not realistic — that's the point: the stylized-facts
//! battery shows *which* realism Poisson lacks and Hawkes restores.

use rand::Rng as _;
use serde::{Deserialize, Serialize};
use sim_core::{OrderKind, OwnerId, Price, Qty, Request, SimTime, SymbolId, TimeInForce};

use super::{BookView, FlowCtx, OrderFlowSource, OwnOrders, PlacementModel, next_arrival, submit};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PoissonConfig {
    /// Limit-order arrivals per second (both sides combined).
    pub limit_rate: f64,
    /// Market-order arrivals per second (both sides combined).
    pub market_rate: f64,
    /// Cancel attempts per second (of this source's own resting orders).
    pub cancel_rate: f64,
    /// Reference price used until both sides of the book exist.
    pub initial_mid_ticks: i64,
    pub placement: PlacementModel,
}

impl Default for PoissonConfig {
    fn default() -> Self {
        Self {
            limit_rate: 40.0,
            market_rate: 4.0,
            cancel_rate: 12.0,
            initial_mid_ticks: 10_000,
            placement: PlacementModel::default(),
        }
    }
}

/// The three independent Poisson clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Limit,
    Market,
    Cancel,
}

#[derive(Debug)]
pub struct PoissonSource {
    config: PoissonConfig,
    symbol: SymbolId,
    owner: OwnerId,
    own: OwnOrders,
    /// Next arrival per class (absolute virtual time).
    next_limit: Option<SimTime>,
    next_market: Option<SimTime>,
    next_cancel: Option<SimTime>,
}

impl PoissonSource {
    #[must_use]
    pub fn new(config: PoissonConfig, symbol: SymbolId, owner: OwnerId) -> Self {
        Self {
            config,
            symbol,
            owner,
            own: OwnOrders::default(),
            next_limit: None,
            next_market: None,
            next_cancel: None,
        }
    }

    /// Reference price for placement: mid of the touch when both sides
    /// exist, else whichever side exists, else the configured initial mid.
    fn reference_ticks(&self, book: &dyn BookView) -> i64 {
        match (book.best_bid(), book.best_ask()) {
            (Some(b), Some(a)) => i64::midpoint(b.ticks(), a.ticks()),
            (Some(b), None) => b.ticks(),
            (None, Some(a)) => a.ticks(),
            (None, None) => self.config.initial_mid_ticks,
        }
    }

    fn earliest(&self) -> Option<(SimTime, Class)> {
        let mut best: Option<(SimTime, Class)> = None;
        for (at, class) in [
            (self.next_limit, Class::Limit),
            (self.next_market, Class::Market),
            (self.next_cancel, Class::Cancel),
        ]
        .into_iter()
        .filter_map(|(at, c)| at.map(|t| (t, c)))
        {
            match best {
                Some((bt, _)) if bt <= at => {}
                _ => best = Some((at, class)),
            }
        }
        best
    }
}

impl OrderFlowSource for PoissonSource {
    fn next(&mut self, ctx: &mut FlowCtx<'_>) -> Option<(SimTime, Request)> {
        // Initialize clocks lazily on first wake.
        if self.next_limit.is_none() && self.next_market.is_none() && self.next_cancel.is_none() {
            self.next_limit = Some(next_arrival(ctx.now, self.config.limit_rate, ctx.rng));
            self.next_market = Some(next_arrival(ctx.now, self.config.market_rate, ctx.rng));
            self.next_cancel = Some(next_arrival(ctx.now, self.config.cancel_rate, ctx.rng));
        }

        // Retry until a class produces an action (cancel with nothing resting
        // re-arms and tries again) — bounded in practice by arrival spacing.
        loop {
            let (at, class) = self.earliest()?;
            match class {
                Class::Limit => {
                    self.next_limit = Some(next_arrival(at, self.config.limit_rate, ctx.rng));
                    let side = if ctx.rng.random::<bool>() {
                        sim_core::Side::Buy
                    } else {
                        sim_core::Side::Sell
                    };
                    let reference = self.reference_ticks(ctx.book);
                    let depth = self.config.placement.depth_ticks(ctx.rng);
                    let price = match side {
                        sim_core::Side::Buy => reference - depth,
                        sim_core::Side::Sell => reference + depth,
                    };
                    if price < 1 {
                        continue; // placement fell off the grid; skip this arrival
                    }
                    let qty = Qty::new(self.config.placement.size_lots(ctx.rng));
                    let (_, request) = submit(
                        ctx.id_gen,
                        self.owner,
                        self.symbol,
                        side,
                        OrderKind::Limit(Price::new(price)),
                        qty,
                        TimeInForce::Gtc,
                    );
                    return Some((at, request));
                }
                Class::Market => {
                    self.next_market = Some(next_arrival(at, self.config.market_rate, ctx.rng));
                    let side = if ctx.rng.random::<bool>() {
                        sim_core::Side::Buy
                    } else {
                        sim_core::Side::Sell
                    };
                    let qty = Qty::new(self.config.placement.size_lots(ctx.rng));
                    let (_, request) = submit(
                        ctx.id_gen,
                        self.owner,
                        self.symbol,
                        side,
                        OrderKind::Market,
                        qty,
                        TimeInForce::Ioc,
                    );
                    return Some((at, request));
                }
                Class::Cancel => {
                    self.next_cancel = Some(next_arrival(at, self.config.cancel_rate, ctx.rng));
                    let pick = ctx.rng.random::<u64>() as usize;
                    if let Some(id) = self.own.pick(pick) {
                        return Some((at, Request::Cancel { id }));
                    }
                    // Nothing resting: consume the arrival and keep looking.
                }
            }
        }
    }

    fn on_own_record(&mut self, record: &sim_core::EventRecord) {
        self.own.on_record(record);
    }
}
