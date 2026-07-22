//! `quant-sim run`: execute a scenario (or a study sweep), report, verify.

use std::path::{Path, PathBuf};

use backtester::strategy::StrategyActor;
use backtester::{ReportSpec, RunReport, StudyReport, build_report, build_study};
use market_sim::data::read_trades_csv;
use market_sim::log::{LogHeader, write_log};
use market_sim::source::coint_pair::CointPairSource;
use market_sim::source::hawkes::HawkesSource;
use market_sim::source::poisson::PoissonSource;
use market_sim::source::replay::ReplaySource;
use market_sim::{SimConfig, Simulation};
use sim_core::{OwnerId, SimDuration, SimTime, SymbolId};
use strategies::{AlmgrenChriss, AvellanedaStoikov, NaiveMm, OuPairs, Twap};

use crate::scenario::{FlowCfg, LoadedScenario, Scenario, StrategyCfg, apply_sweep, load};

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Scenario(#[from] crate::scenario::ScenarioError),
    #[error("log: {0}")]
    Log(#[from] market_sim::log::LogError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event-stream hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Wire one flow configuration into the simulation.
fn add_flow(sim: &mut Simulation, loaded: &LoadedScenario, flow: &FlowCfg) {
    match flow {
        FlowCfg::Poisson {
            symbol,
            owner,
            order_latency,
            poisson,
        } => {
            let symbol_id = loaded.symbols[symbol];
            sim.add_source(
                symbol_id,
                OwnerId::new(*owner),
                Box::new(PoissonSource::new(
                    *poisson,
                    symbol_id,
                    OwnerId::new(*owner),
                )),
                order_latency,
            );
        }
        FlowCfg::Hawkes {
            symbol,
            owner,
            order_latency,
            hawkes,
            fit_file,
        } => {
            let symbol_id = loaded.symbols[symbol];
            let mut config = hawkes.clone();
            if let Some(path) = fit_file {
                // Override the market buy/sell dims from a fitted 2-dim
                // params artifact (docs/interchange.md).
                let params = market_sim::hawkes::read_params(std::path::Path::new(path))
                    .unwrap_or_else(|e| panic!("fit_file {path}: {e}"));
                config.mu[0] = params.baseline.mu[0];
                config.mu[1] = params.baseline.mu[1];
                config.beta = params.kernel.beta;
                for (row, fitted) in config.alpha.iter_mut().zip(&params.kernel.alpha).take(2) {
                    for (a, f) in row.iter_mut().zip(fitted).take(2) {
                        *a = *f;
                    }
                }
            }
            let source = HawkesSource::new(config, symbol_id, OwnerId::new(*owner))
                .unwrap_or_else(|e| panic!("hawkes flow: {e}"));
            sim.add_source(
                symbol_id,
                OwnerId::new(*owner),
                Box::new(source),
                order_latency,
            );
        }
        FlowCfg::Replay {
            symbol,
            owner,
            taker_owner,
            trades_csv,
            replay,
        } => {
            let symbol_id = loaded.symbols[symbol];
            let rows = read_trades_csv(std::path::Path::new(trades_csv))
                .unwrap_or_else(|e| panic!("trades_csv {trades_csv}: {e}"));
            let instrument = &loaded.instruments[&symbol_id];
            let source = ReplaySource::new(
                &rows,
                instrument,
                *replay,
                symbol_id,
                OwnerId::new(*owner),
                OwnerId::new(*taker_owner),
                SimDuration::from_secs(1),
            )
            .unwrap_or_else(|e| panic!("replay flow: {e}"));
            // Replay requires a zero-latency link (tape order is sacred).
            sim.add_source(
                symbol_id,
                OwnerId::new(*owner),
                Box::new(source),
                &market_sim::LatencySpec::Zero,
            );
        }
        FlowCfg::CointPair { owner, coint } => {
            let source = CointPairSource::new(*coint, OwnerId::new(*owner));
            // The source drives both legs; register it under leg A's engine.
            sim.add_source(
                SymbolId::new(coint.symbol_a),
                OwnerId::new(*owner),
                Box::new(source),
                &market_sim::LatencySpec::Zero,
            );
        }
    }
}

/// Wire one strategy into the simulation; returns its (name, owner) for
/// report tracking.
fn add_strategy(
    sim: &mut Simulation,
    loaded: &LoadedScenario,
    strategy: &StrategyCfg,
) -> (String, OwnerId) {
    let owner_id = OwnerId::new(strategy.owner());
    let name = strategy.name().to_string();
    match strategy {
        StrategyCfg::NaiveMm {
            order_latency,
            md_latency,
            params,
            ..
        } => {
            sim.add_actor(
                owner_id,
                [SymbolId::new(params.symbol_index)],
                Box::new(StrategyActor::new(Box::new(NaiveMm::new(*params)))),
                order_latency,
                md_latency,
            );
        }
        StrategyCfg::AvellanedaStoikov {
            order_latency,
            md_latency,
            params,
            ..
        } => {
            sim.add_actor(
                owner_id,
                [SymbolId::new(params.symbol_index)],
                Box::new(StrategyActor::new(Box::new(AvellanedaStoikov::new(
                    *params,
                )))),
                order_latency,
                md_latency,
            );
        }
        StrategyCfg::OuPairs {
            order_latency,
            md_latency,
            symbols,
            params,
            ..
        } => {
            let subs: Vec<SymbolId> = symbols.iter().map(|s| loaded.symbols[s]).collect();
            sim.add_actor(
                owner_id,
                subs,
                Box::new(StrategyActor::new(Box::new(OuPairs::new(*params)))),
                order_latency,
                md_latency,
            );
        }
        StrategyCfg::AlmgrenChriss {
            order_latency,
            md_latency,
            params,
            ..
        } => {
            sim.add_actor(
                owner_id,
                [SymbolId::new(params.exec.symbol_index)],
                Box::new(StrategyActor::new(Box::new(AlmgrenChriss::new(*params)))),
                order_latency,
                md_latency,
            );
        }
        StrategyCfg::Twap {
            order_latency,
            md_latency,
            params,
            ..
        } => {
            sim.add_actor(
                owner_id,
                [SymbolId::new(params.exec.symbol_index)],
                Box::new(StrategyActor::new(Box::new(Twap::new(*params)))),
                order_latency,
                md_latency,
            );
        }
    }
    (name, owner_id)
}

/// Build and run one simulation from a (possibly sweep-modified) scenario.
fn run_once(loaded: &LoadedScenario, scenario: &Scenario, seed: u64) -> (Simulation, RunReport) {
    let mut sim = Simulation::new(SimConfig {
        seed,
        t_end: SimTime::from_nanos(
            u64::try_from(scenario.run.t_end.as_nanos()).expect("t_end fits u64 ns"),
        ),
    });
    for &symbol_id in loaded.instruments.keys() {
        sim.add_engine(symbol_id, loaded.engine_config());
    }
    for flow in &scenario.flows {
        add_flow(&mut sim, loaded, flow);
    }
    let mut tracked = Vec::new();
    for strategy in &scenario.strategies {
        let (name, owner_id) = add_strategy(&mut sim, loaded, strategy);
        tracked.push((name, owner_id));
    }
    sim.run();

    let spec = ReportSpec {
        scenario_hash: loaded.hash_hex.clone(),
        sample_every: SimDuration::from_nanos(
            i64::try_from(scenario.run.equity_sample.as_nanos()).expect("sample fits"),
        ),
        capital_base_e8: loaded.capital_base_e8(),
        strategies: tracked,
    };
    let report = build_report(&sim, &loaded.instruments, &scenario.fees.to_model(), &spec);
    (sim, report)
}

pub struct RunArgs {
    pub scenario: PathBuf,
    pub seed: Option<u64>,
    pub out: Option<PathBuf>,
    pub format: OutputFormat,
    pub verify_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Both,
}

pub fn run(args: &RunArgs) -> Result<(), RunError> {
    let loaded = load(&args.scenario)?;
    let base_seed = args.seed.unwrap_or(loaded.scenario.run.seed);

    if let Some(study_cfg) = loaded.scenario.study.clone() {
        // Study: sweep variants, identical seed (paired background flow).
        let mut runs: Vec<RunReport> = Vec::new();
        let pick = loaded
            .scenario
            .strategies
            .iter()
            .position(|s| s.name() == study_cfg.strategy)
            .expect("validated");
        for value in &study_cfg.values {
            let mut variant = loaded.scenario.clone();
            apply_sweep(&mut variant, &study_cfg.strategy, &study_cfg.param, value)?;
            let label = format!("{}[{}={}]", study_cfg.strategy, study_cfg.param, value);
            let (_, mut report) = run_once(&loaded, &variant, base_seed);
            if let Some(block) = report.strategies.get_mut(pick) {
                block.name = label;
            }
            runs.push(report);
        }
        let study = build_study(runs, pick);
        emit_study(&study, args)?;
        return Ok(());
    }

    let (sim, report) = run_once(&loaded, &loaded.scenario, base_seed);
    if let Some(expected) = &args.verify_hash {
        let actual = report.events_hash.clone();
        if !expected.eq_ignore_ascii_case(&actual) {
            return Err(RunError::HashMismatch {
                expected: expected.clone(),
                actual,
            });
        }
        eprintln!("verify-hash: OK ({actual})");
    }
    if let Some(out) = &args.out {
        std::fs::create_dir_all(out)?;
        write_log(
            &out.join("events.qsim"),
            &LogHeader {
                version: 1,
                seed: base_seed,
                scenario_hash: loaded.hash_hex.clone(),
            },
            &sim.log.records,
        )?;
        std::fs::write(
            out.join("report.json"),
            format!("{}\n", serde_json::to_string_pretty(&report)?),
        )?;
        eprintln!("wrote {}", out.display());
    }
    emit_run(&report, args)?;
    Ok(())
}

fn emit_run(report: &RunReport, args: &RunArgs) -> Result<(), RunError> {
    if matches!(args.format, OutputFormat::Table | OutputFormat::Both) {
        eprintln!("{}", crate::table::run_table(report));
    }
    if matches!(args.format, OutputFormat::Json | OutputFormat::Both) {
        println!("{}", serde_json::to_string(report)?);
    }
    Ok(())
}

fn emit_study(study: &StudyReport, args: &RunArgs) -> Result<(), RunError> {
    if matches!(args.format, OutputFormat::Table | OutputFormat::Both) {
        eprintln!("{}", crate::table::study_table(study));
    }
    if matches!(args.format, OutputFormat::Json | OutputFormat::Both) {
        println!("{}", serde_json::to_string(study)?);
    }
    if let Some(out) = &args.out {
        std::fs::create_dir_all(out)?;
        std::fs::write(
            out.join("study.json"),
            format!("{}\n", serde_json::to_string_pretty(study)?),
        )?;
        eprintln!("wrote {}", out.display());
    }
    Ok(())
}

/// `quant-sim scenario validate`.
pub fn validate(path: &Path) -> Result<(), RunError> {
    let loaded = load(path)?;
    eprintln!(
        "ok: {} — {} ({} instruments, {} flows, {} strategies) hash {}",
        loaded.scenario.meta.name,
        loaded.scenario.meta.description,
        loaded.instruments.len(),
        loaded.scenario.flows.len(),
        loaded.scenario.strategies.len(),
        loaded.hash_hex
    );
    Ok(())
}
