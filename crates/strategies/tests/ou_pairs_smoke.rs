//! OU pairs on the synthetic cointegrated source must actually trade.

use std::collections::BTreeMap;

use backtester::strategy::StrategyActor;
use backtester::{FeeModel, ReportSpec, build_report};
use market_sim::source::coint_pair::{CointPairConfig, CointPairSource};
use market_sim::{LatencySpec, SimConfig, Simulation};
use matching_engine::EngineConfig;
use sim_core::{EngineEvent, Instrument, OwnerId, SimDuration, SimTime, SymbolId};
use strategies::{OuPairs, OuPairsConfig};

const A: SymbolId = SymbolId::new(0);
const B: SymbolId = SymbolId::new(1);
const FLOW: OwnerId = OwnerId::new(0);
const STRAT: OwnerId = OwnerId::new(1);

#[test]
fn ou_pairs_trades_the_synthetic_pair() {
    let mut sim = Simulation::new(SimConfig {
        seed: 42,
        t_end: SimTime::from_secs(1800),
    });
    sim.add_engine(A, EngineConfig::default());
    sim.add_engine(B, EngineConfig::default());
    sim.add_source(
        A,
        FLOW,
        Box::new(CointPairSource::new(CointPairConfig::default(), FLOW)),
        &LatencySpec::Zero,
    );
    let params = OuPairsConfig {
        window: 300,
        refit_every: 120,
        ..OuPairsConfig::default()
    };
    sim.add_actor(
        STRAT,
        [A, B],
        Box::new(StrategyActor::new(Box::new(OuPairs::new(params)))),
        &LatencySpec::Zero,
        &LatencySpec::Zero,
    );
    sim.run();

    // The synthetic pair produced a two-sided book on both legs.
    let mut events_a = 0;
    let mut events_leg_two = 0;
    let mut strat_fills = 0;
    for record in &sim.log.records {
        match record.symbol {
            s if s == A => events_a += 1,
            s if s == B => events_leg_two += 1,
            _ => {}
        }
        if let EngineEvent::Fill {
            maker_owner,
            taker_owner,
            ..
        } = record.event
            && (maker_owner == STRAT || taker_owner == STRAT)
        {
            strat_fills += 1;
        }
    }
    assert!(events_a > 100, "leg A had {events_a} events");
    assert!(events_leg_two > 100, "leg B had {events_leg_two} events");
    assert!(
        strat_fills > 0,
        "OU pairs strategy never filled ({strat_fills})"
    );

    let mut instruments = BTreeMap::new();
    instruments.insert(
        A,
        Instrument::new("PAIR-A", A, 1_000_000, 100_000_000).unwrap(),
    );
    instruments.insert(
        B,
        Instrument::new("PAIR-B", B, 1_000_000, 100_000_000).unwrap(),
    );
    let spec = ReportSpec {
        scenario_hash: "test".into(),
        sample_every: SimDuration::from_secs(1),
        capital_base_e8: 1_000_000 * 100_000_000,
        strategies: vec![("ou_pairs".into(), STRAT)],
    };
    let report = build_report(&sim, &instruments, &FeeModel::default(), &spec);
    assert!(report.strategies[0].trade_stats.fills > 0);
}
