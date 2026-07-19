//! Run/study report artifacts (`schema_version: 1`).
//!
//! JSON contract shared with the Python research layer
//! (`docs/interchange.md`): the per-period `returns` array is the
//! authoritative series the cross-check recomputes metrics from.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlBlock {
    pub realized_e8: i128,
    pub unrealized_e8: i128,
    pub fees_e8: i128,
    pub equity_end_e8: i128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsBlock {
    /// Per-period Sharpe (no annualization) and its annualized display twin.
    pub sharpe: Option<f64>,
    pub sharpe_annualized: Option<f64>,
    pub sortino: Option<f64>,
    /// Probabilistic Sharpe vs SR* = 0.
    pub psr: Option<f64>,
    /// Deflated Sharpe: only computable from a study (K ≥ 2 real trials).
    pub dsr: Option<f64>,
    /// Why DSR is absent, when it is (e.g. "K=1").
    pub dsr_reason: Option<String>,
    pub max_drawdown: f64,
    pub drawdown_len_samples: usize,
    pub n_periods: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeStats {
    pub fills: u64,
    pub maker_fills: u64,
    pub gross_traded_e8: i128,
    pub position_end_lots: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyReport {
    pub name: String,
    pub owner: u16,
    pub pnl: PnlBlock,
    pub metrics: MetricsBlock,
    pub trade_stats: TradeStats,
    /// Equity samples on the fixed virtual-time grid, e8 (relative to 0).
    pub equity_e8: Vec<i128>,
    /// Per-period simple returns on `capital_base` — the authoritative
    /// series for all statistics.
    pub returns: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub schema_version: u32,
    pub seed: u64,
    pub scenario_hash: String,
    /// BLAKE3 of the run's event stream.
    pub events_hash: String,
    pub event_count: usize,
    pub sim_span_ns: u64,
    /// Equity sampling period.
    pub period_ns: u64,
    /// Base capital used to convert equity deltas into returns (e8). A
    /// reporting convention, stated rather than hidden.
    pub capital_base_e8: i128,
    pub strategies: Vec<StrategyReport>,
}

/// A study = the same scenario swept over parameter variants. The DSR of the
/// selected (best) variant is computed against **these** trials — the honest
/// K, not an imagined one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyReport {
    pub schema_version: u32,
    pub n_trials: usize,
    pub trial_names: Vec<String>,
    pub trial_sharpes: Vec<f64>,
    pub best_index: usize,
    /// DSR of the best variant given all trials.
    pub dsr_best: Option<f64>,
    pub runs: Vec<RunReport>,
}
