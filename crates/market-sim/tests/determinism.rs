//! Simulation-level determinism (P14 at full-system scale) and projection
//! losslessness (P13 across a whole Poisson run).

use market_sim::source::poisson::{PoissonConfig, PoissonSource};
use market_sim::{BookProjection, LatencySpec, SimConfig, Simulation};
use matching_engine::{Book as _, EngineConfig};
use sim_core::{OwnerId, Side, SimTime, SymbolId};

const SYM: SymbolId = SymbolId::new(0);

fn run(seed: u64, latency: &LatencySpec, seconds: u64) -> Simulation {
    let mut sim = Simulation::new(SimConfig {
        seed,
        t_end: SimTime::from_secs(seconds),
    });
    sim.add_engine(SYM, EngineConfig::default());
    sim.add_source(
        SYM,
        OwnerId::new(0),
        Box::new(PoissonSource::new(
            PoissonConfig::default(),
            SYM,
            OwnerId::new(0),
        )),
        latency,
    );
    sim.run();
    sim
}

#[test]
fn same_seed_is_byte_identical() {
    let latency = LatencySpec::LogNormal {
        median_ns: 200_000,
        sigma: 0.4,
        min_ns: 20_000,
    };
    let a = run(42, &latency, 30);
    let b = run(42, &latency, 30);
    assert!(!a.log.is_empty(), "simulation produced no events");
    assert_eq!(
        a.log.hash_hex(),
        b.log.hash_hex(),
        "same seed must reproduce exactly"
    );
    assert_eq!(a.log.len(), b.log.len());
}

#[test]
fn different_seeds_diverge() {
    let latency = LatencySpec::Zero;
    let a = run(1, &latency, 10);
    let b = run(2, &latency, 10);
    assert_ne!(a.log.hash_hex(), b.log.hash_hex());
}

#[test]
fn projection_matches_engine_book_after_full_run() {
    let latency = LatencySpec::Constant { nanos: 50_000 };
    let sim = run(7, &latency, 30);

    let mut projection = BookProjection::new();
    for record in &sim.log.records {
        projection.apply(record);
    }
    let engine = sim.engine(SYM).expect("engine exists");
    let snapshot = engine.book().snapshot();

    let mut engine_orders = 0usize;
    for (side, levels) in [(Side::Buy, &snapshot.bids), (Side::Sell, &snapshot.asks)] {
        for (price, orders) in levels {
            let total: u128 = orders.iter().map(|o| u128::from(o.remaining.lots())).sum();
            let count = u32::try_from(orders.len()).expect("fits");
            assert_eq!(
                projection.level(side, *price),
                Some((total, count)),
                "projection level mismatch at {price} ({side:?})"
            );
            engine_orders += orders.len();
        }
    }
    assert_eq!(projection.resting_orders(), engine_orders);
    assert_eq!(
        projection.best_bid().map(|(p, _)| p),
        snapshot.best_bid(),
        "best bid"
    );
    assert_eq!(
        projection.best_ask().map(|(p, _)| p),
        snapshot.best_ask(),
        "best ask"
    );
}

/// Golden system-level hash: scenario + seed pinned; any change to RNG use,
/// scheduler ordering, source logic or engine semantics changes this value
/// and must be a conscious, explained commit.
#[test]
fn golden_run_hash_is_stable() {
    let latency = LatencySpec::LogNormal {
        median_ns: 200_000,
        sigma: 0.4,
        min_ns: 20_000,
    };
    let sim = run(42, &latency, 10);
    assert_eq!(
        sim.log.hash_hex(),
        "32aa58c4df19a3146f58dac6fbbe62b7f2da7dc99a85b4b89a10c94675d0b568",
        "system-level event stream changed; if intentional, regenerate this constant"
    );
}
