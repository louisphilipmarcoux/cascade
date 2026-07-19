//! Deterministic benchmark workloads (shared by the criterion throughput
//! bench and the hdrhistogram latency soak via `#[path]` include).
//!
//! Three shapes, all seeded and reproducible:
//! - `uniform`: submits spread over a wide price band, moderate cancels —
//!   stresses level creation/removal (`BTreeMap` churn);
//! - `near_touch`: geometric price concentration at the touch plus a high
//!   cancel/replace rate — the HFT-shaped workload;
//! - `sweepy`: frequent large marketable orders — stresses the matching loop
//!   and multi-level sweeps.

use rand::Rng as _;
use sim_core::rng::{Rng, SimRng, StreamId};
use sim_core::{
    NewOrder, OrderId, OrderKind, OwnerId, Price, Qty, Request, Side, SymbolId, TimeInForce,
};

pub const SYM: SymbolId = SymbolId::new(0);

#[derive(Debug, Clone, Copy)]
pub struct WorkloadSpec {
    pub mid: i64,
    pub band: i64,
    /// Geometric-ish concentration: probability of widening one tick.
    pub widen_p: f64,
    pub cancel_ratio: f64,
    pub market_ratio: f64,
    pub size_max: u64,
}

pub const UNIFORM: WorkloadSpec = WorkloadSpec {
    mid: 10_000,
    band: 500,
    widen_p: 1.0, // uniform over the band
    cancel_ratio: 0.25,
    market_ratio: 0.05,
    size_max: 100,
};

pub const NEAR_TOUCH: WorkloadSpec = WorkloadSpec {
    mid: 10_000,
    band: 200,
    widen_p: 0.35,
    cancel_ratio: 0.45,
    market_ratio: 0.05,
    size_max: 50,
};

pub const SWEEPY: WorkloadSpec = WorkloadSpec {
    mid: 10_000,
    band: 100,
    widen_p: 0.5,
    cancel_ratio: 0.15,
    market_ratio: 0.25,
    size_max: 200,
};

/// Pre-generates `n` requests (generation cost stays out of the timed loop).
pub fn generate(spec: &WorkloadSpec, seed: u64, n: usize) -> Vec<Request> {
    let mut rng: Rng = SimRng::new(seed).stream(StreamId::FLOW_SOURCE);
    let mut requests = Vec::with_capacity(n);
    let mut next_id: u64 = 0;
    let mut live: Vec<OrderId> = Vec::new();

    for _ in 0..n {
        let roll: f64 = rng.random();
        if roll < spec.cancel_ratio && !live.is_empty() {
            let idx = rng.random_range(0..live.len());
            let id = live.swap_remove(idx);
            requests.push(Request::Cancel { id });
            continue;
        }
        let side = if rng.random::<bool>() {
            Side::Buy
        } else {
            Side::Sell
        };
        let qty = Qty::new(rng.random_range(1..=spec.size_max));
        let id = OrderId::new(next_id);
        next_id += 1;
        let (kind, tif) = if roll > 1.0 - spec.market_ratio {
            (OrderKind::Market, TimeInForce::Ioc)
        } else {
            // Distance from mid: geometric via repeated widening, capped.
            let mut distance: i64 = 1;
            while distance < spec.band && rng.random::<f64>() < spec.widen_p {
                distance += 1;
            }
            if spec.widen_p >= 1.0 {
                distance = rng.random_range(1..=spec.band);
            }
            let price = match side {
                Side::Buy => spec.mid - distance,
                Side::Sell => spec.mid + distance,
            };
            live.push(id);
            (OrderKind::Limit(Price::new(price)), TimeInForce::Gtc)
        };
        requests.push(Request::Submit(NewOrder {
            id,
            owner: OwnerId::new((next_id % 5) as u16),
            symbol: SYM,
            side,
            kind,
            qty,
            tif,
        }));
    }
    requests
}
