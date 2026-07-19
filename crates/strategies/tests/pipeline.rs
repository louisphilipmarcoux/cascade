//! The end-to-end pipeline proof (M5 milestone gate): Poisson background
//! flow + a latency-honest naive market maker, through the real engine, into
//! accounting, metrics and a run report — deterministic and internally
//! consistent.

use std::collections::BTreeMap;

use backtester::strategy::StrategyActor;
use backtester::{FeeModel, ReportSpec, build_report};
use market_sim::source::poisson::{PoissonConfig, PoissonSource};
use market_sim::{LatencySpec, SimConfig, Simulation};
use matching_engine::EngineConfig;
use sim_core::{Instrument, OwnerId, SimDuration, SimTime, SymbolId};
use strategies::{NaiveMm, NaiveMmConfig};

const SYM: SymbolId = SymbolId::new(0);
const FLOW_OWNER: OwnerId = OwnerId::new(0);
const MM_OWNER: OwnerId = OwnerId::new(1);

fn run_pipeline(seed: u64) -> (backtester::RunReport, String) {
    let mut sim = Simulation::new(SimConfig {
        seed,
        t_end: SimTime::from_secs(120),
    });
    sim.add_engine(SYM, EngineConfig::default());
    sim.add_source(
        SYM,
        FLOW_OWNER,
        Box::new(PoissonSource::new(
            PoissonConfig::default(),
            SYM,
            FLOW_OWNER,
        )),
        &LatencySpec::Zero,
    );
    let mm = NaiveMm::new(NaiveMmConfig::default());
    sim.add_actor(
        MM_OWNER,
        [SYM],
        Box::new(StrategyActor::new(Box::new(mm))),
        &LatencySpec::LogNormal {
            median_ns: 200_000,
            sigma: 0.4,
            min_ns: 20_000,
        },
        &LatencySpec::Constant { nanos: 50_000 },
    );
    sim.run();

    let mut instruments = BTreeMap::new();
    instruments.insert(
        SYM,
        Instrument::new("SYNTH", SYM, 1_000_000, 100_000_000).unwrap(),
    );
    let spec = ReportSpec {
        scenario_hash: "test".into(),
        sample_every: SimDuration::from_secs(1),
        capital_base_e8: 1_000_000 * 100_000_000,
        strategies: vec![("naive_mm".into(), MM_OWNER)],
    };
    let report = build_report(&sim, &instruments, &FeeModel::default(), &spec);
    let hash = sim.log.hash_hex();
    (report, hash)
}

#[test]
fn pipeline_produces_consistent_report() {
    let (report, _) = run_pipeline(42);
    assert_eq!(report.schema_version, 1);
    assert_eq!(report.strategies.len(), 1);
    let mm = &report.strategies[0];

    // The MM actually traded through the real engine.
    assert!(mm.trade_stats.fills > 0, "market maker never filled");
    assert!(mm.trade_stats.maker_fills > 0, "market maker never made");
    assert!(mm.metrics.n_periods > 100, "equity grid too sparse");

    // P19 at report level: equity_end == realized + unrealized − fees.
    assert_eq!(
        mm.pnl.equity_end_e8,
        mm.pnl.realized_e8 + mm.pnl.unrealized_e8 - mm.pnl.fees_e8,
        "accounting identity broken in report"
    );
    // Fees are nonzero (it traded) and DSR is honestly absent for K=1.
    assert!(mm.pnl.fees_e8 != 0);
    assert_eq!(mm.metrics.dsr, None);
    assert_eq!(mm.metrics.dsr_reason.as_deref(), Some("K=1"));
    // Equity series and returns line up.
    assert_eq!(mm.returns.len() + 1, mm.equity_e8.len());
}

#[test]
fn pipeline_is_deterministic_end_to_end() {
    let (report_a, hash_a) = run_pipeline(7);
    let (report_b, hash_b) = run_pipeline(7);
    assert_eq!(hash_a, hash_b, "event streams diverged");
    assert_eq!(
        serde_json::to_string(&report_a).unwrap(),
        serde_json::to_string(&report_b).unwrap(),
        "reports diverged"
    );
    let (_, hash_c) = run_pipeline(8);
    assert_ne!(hash_a, hash_c, "different seeds must differ");
}
