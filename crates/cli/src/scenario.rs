//! Scenario files: the single, hashed description of a run.
//!
//! Everything that affects the event stream lives here; the BLAKE3 hash of
//! the file bytes goes into the run report, and `run --verify-hash` re-runs
//! and compares event-stream hashes — a one-command determinism proof.

use std::collections::BTreeMap;
use std::path::Path;

use market_sim::LatencySpec;
use market_sim::source::hawkes::HawkesFlowConfig;
use market_sim::source::poisson::PoissonConfig;
pub use market_sim::source::replay::ReplayConfig as ReplayCfg;
use matching_engine::SelfMatchPolicy;
use serde::Deserialize;
use sim_core::instrument::parse_decimal_e8;
use sim_core::{Instrument, Qty, SymbolId};
use strategies::NaiveMmConfig;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub meta: Meta,
    pub run: RunCfg,
    #[serde(rename = "instrument")]
    pub instruments: Vec<InstrumentCfg>,
    #[serde(default)]
    pub engine: EngineCfg,
    #[serde(rename = "flow")]
    pub flows: Vec<FlowCfg>,
    #[serde(rename = "strategy", default)]
    pub strategies: Vec<StrategyCfg>,
    #[serde(default)]
    pub fees: FeesCfg,
    pub study: Option<StudyCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCfg {
    pub seed: u64,
    /// e.g. "120s", "4h".
    #[serde(with = "humantime_serde")]
    pub t_end: std::time::Duration,
    /// Equity sampling period (default 1s).
    #[serde(with = "humantime_serde", default = "default_sample")]
    pub equity_sample: std::time::Duration,
    /// Base capital for return scaling, quote currency (decimal string).
    pub capital_base: String,
}

fn default_sample() -> std::time::Duration {
    std::time::Duration::from_secs(1)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentCfg {
    pub symbol: String,
    /// Quote per tick, decimal string (e.g. "0.01").
    pub tick_size: String,
    /// Base per lot, decimal string (e.g. "1.0").
    pub lot_size: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EngineCfg {
    pub self_match: SelfMatchPolicy,
    pub max_order_qty: u64,
}

impl Default for EngineCfg {
    fn default() -> Self {
        Self {
            self_match: SelfMatchPolicy::default(),
            max_order_qty: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowCfg {
    Poisson {
        symbol: String,
        owner: u16,
        #[serde(default = "zero_latency")]
        order_latency: LatencySpec,
        /// Rates + placement model ([`PoissonConfig`] carries both).
        #[serde(default)]
        poisson: PoissonConfig,
    },
    Hawkes {
        symbol: String,
        owner: u16,
        #[serde(default = "zero_latency")]
        order_latency: LatencySpec,
        /// Inline 4-dim flow parameters; alternatively `fit_file` overrides
        /// the market buy/sell dims from a fitted 2-dim params TOML.
        #[serde(default)]
        hawkes: HawkesFlowConfig,
        #[serde(default)]
        fit_file: Option<String>,
    },
    Replay {
        symbol: String,
        /// Maker/frame owner; the aggressor uses `taker_owner`.
        owner: u16,
        taker_owner: u16,
        /// Trades interchange CSV (docs/interchange.md), repo-relative or
        /// absolute.
        trades_csv: String,
        #[serde(default)]
        replay: ReplayCfg,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrategyCfg {
    NaiveMm {
        name: String,
        owner: u16,
        #[serde(default = "default_order_latency")]
        order_latency: LatencySpec,
        #[serde(default = "default_md_latency")]
        md_latency: LatencySpec,
        #[serde(default)]
        params: NaiveMmConfig,
    },
}

impl StrategyCfg {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::NaiveMm { name, .. } => name,
        }
    }

    #[must_use]
    pub const fn owner(&self) -> u16 {
        match self {
            Self::NaiveMm { owner, .. } => *owner,
        }
    }
}

const fn zero_latency() -> LatencySpec {
    LatencySpec::Zero
}

const fn default_order_latency() -> LatencySpec {
    LatencySpec::LogNormal {
        median_ns: 200_000,
        sigma: 0.4,
        min_ns: 20_000,
    }
}

const fn default_md_latency() -> LatencySpec {
    LatencySpec::Constant { nanos: 50_000 }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FeesCfg {
    pub maker_bps: f64,
    pub taker_bps: f64,
}

impl Default for FeesCfg {
    fn default() -> Self {
        Self {
            maker_bps: 1.0,
            taker_bps: 5.0,
        }
    }
}

impl FeesCfg {
    #[must_use]
    pub fn to_model(self) -> backtester::FeeModel {
        backtester::FeeModel {
            maker_rate_e8: bps_to_rate_e8(self.maker_bps),
            taker_rate_e8: bps_to_rate_e8(self.taker_bps),
        }
    }
}

/// 1 bp = 1e-4 → `rate_e8` = bps × `10_000` (rounded; sub-1e-8 rates refuse).
fn bps_to_rate_e8(bps: f64) -> i64 {
    let rate = bps * 10_000.0;
    let rounded = libm::round(rate);
    assert!(
        (rate - rounded).abs() < 1e-6,
        "fee of {bps} bps is not representable at e8 scale"
    );
    rounded as i64
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyCfg {
    /// Which strategy (by name) is swept — its variants form the trial set.
    pub strategy: String,
    /// Parameter name within that strategy's `params` table.
    pub param: String,
    pub values: Vec<toml::Value>,
}

/// A scenario parsed together with its provenance.
#[derive(Debug, Clone)]
pub struct LoadedScenario {
    pub scenario: Scenario,
    /// BLAKE3 of the file bytes.
    pub hash_hex: String,
    /// Symbol name → dense id (declaration order).
    pub symbols: BTreeMap<String, SymbolId>,
    pub instruments: BTreeMap<SymbolId, Instrument>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

pub fn load(path: &Path) -> Result<LoadedScenario, ScenarioError> {
    let bytes = std::fs::read(path)?;
    let hash_hex = blake3::hash(&bytes).to_hex().to_string();
    let text = String::from_utf8_lossy(&bytes);
    let scenario: Scenario = toml::from_str(&text)?;
    validate(&scenario)?;

    let mut symbols = BTreeMap::new();
    let mut instruments = BTreeMap::new();
    for (index, cfg) in scenario.instruments.iter().enumerate() {
        let id = SymbolId::new(u16::try_from(index).expect("instrument count fits u16"));
        let tick = parse_decimal_e8(&cfg.tick_size).map_err(ScenarioError::Invalid)?;
        let lot = parse_decimal_e8(&cfg.lot_size).map_err(ScenarioError::Invalid)?;
        let instrument = Instrument::new(cfg.symbol.clone(), id, tick, lot)
            .map_err(|e| ScenarioError::Invalid(e.to_string()))?;
        symbols.insert(cfg.symbol.clone(), id);
        instruments.insert(id, instrument);
    }

    Ok(LoadedScenario {
        scenario,
        hash_hex,
        symbols,
        instruments,
    })
}

fn validate(s: &Scenario) -> Result<(), ScenarioError> {
    let invalid = |msg: String| Err(ScenarioError::Invalid(msg));
    if s.instruments.is_empty() {
        return invalid("at least one [[instrument]] is required".into());
    }
    if s.flows.is_empty() {
        return invalid("at least one [[flow]] is required".into());
    }
    let known: Vec<&str> = s.instruments.iter().map(|i| i.symbol.as_str()).collect();
    let mut owners = std::collections::BTreeSet::new();
    for flow in &s.flows {
        let (symbol, flow_owners): (&str, Vec<u16>) = match flow {
            FlowCfg::Poisson { symbol, owner, .. } | FlowCfg::Hawkes { symbol, owner, .. } => {
                (symbol, vec![*owner])
            }
            FlowCfg::Replay {
                symbol,
                owner,
                taker_owner,
                ..
            } => {
                if owner == taker_owner {
                    return invalid("replay owner and taker_owner must differ".into());
                }
                (symbol, vec![*owner, *taker_owner])
            }
        };
        if !known.contains(&symbol) {
            return invalid(format!("flow references unknown symbol {symbol:?}"));
        }
        for owner in flow_owners {
            if !owners.insert(owner) {
                return invalid(format!("owner {owner} used twice"));
            }
        }
    }
    let mut names = std::collections::BTreeSet::new();
    for strategy in &s.strategies {
        if !owners.insert(strategy.owner()) {
            return invalid(format!("owner {} used twice", strategy.owner()));
        }
        if !names.insert(strategy.name().to_string()) {
            return invalid(format!("strategy name {:?} used twice", strategy.name()));
        }
    }
    if let Some(study) = &s.study {
        if study.values.is_empty() {
            return invalid("study.values must be non-empty".into());
        }
        if study.values.len() > 64 {
            return invalid("study.values capped at 64 trials".into());
        }
        if !s.strategies.iter().any(|st| st.name() == study.strategy) {
            return invalid(format!(
                "study sweeps unknown strategy {:?}",
                study.strategy
            ));
        }
    }
    Ok(())
}

/// Apply one sweep value to the named strategy's params (via TOML value
/// round-trip so it generalizes to every strategy kind).
pub fn apply_sweep(
    scenario: &mut Scenario,
    strategy_name: &str,
    param: &str,
    value: &toml::Value,
) -> Result<(), ScenarioError> {
    for strategy in &mut scenario.strategies {
        if strategy.name() != strategy_name {
            continue;
        }
        match strategy {
            StrategyCfg::NaiveMm { params, .. } => {
                let mut table = toml::Value::try_from(*params)
                    .map_err(|e| ScenarioError::Invalid(e.to_string()))?;
                let Some(entry) = table.as_table_mut().and_then(|t| t.get_mut(param)) else {
                    return Err(ScenarioError::Invalid(format!(
                        "strategy {strategy_name:?} has no parameter {param:?}"
                    )));
                };
                *entry = value.clone();
                *params = table
                    .try_into()
                    .map_err(|e: toml::de::Error| ScenarioError::Invalid(e.to_string()))?;
                return Ok(());
            }
        }
    }
    Err(ScenarioError::Invalid(format!(
        "study sweeps unknown strategy {strategy_name:?}"
    )))
}

/// Helpers shared by run/study commands.
impl LoadedScenario {
    #[must_use]
    pub fn capital_base_e8(&self) -> i128 {
        parse_decimal_e8(&self.scenario.run.capital_base).map_or(0, i128::from)
    }

    #[must_use]
    pub fn engine_config(&self) -> matching_engine::EngineConfig {
        matching_engine::EngineConfig {
            self_match: self.scenario.engine.self_match,
            max_order_qty: Qty::new(self.scenario.engine.max_order_qty),
        }
    }
}
