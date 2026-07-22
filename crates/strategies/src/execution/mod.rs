//! Optimal execution: liquidate a fixed position over a horizon, trading off
//! market impact against price risk.
//!
//! Two schedulers share one child-order execution path so their
//! implementation shortfall is compared apples-to-apples under identical
//! frictions and seeds:
//!
//! - [`almgren_chriss`]: the closed-form Almgren–Chriss (2000) trajectory;
//! - [`twap`]: equal slices (time-weighted average price) — the baseline.

pub mod almgren_chriss;
pub mod twap;

use serde::{Deserialize, Serialize};

/// Shared execution configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ExecutionConfig {
    pub symbol_index: u16,
    /// Total lots to liquidate (sign: + sell down a long, − buy back a short).
    pub target_lots: i64,
    /// Number of slices.
    pub slices: u32,
    /// Total horizon in seconds.
    pub horizon_secs: f64,
    /// Delay before the first slice, so execution starts against a populated
    /// book (a synthetic-market artifact: the book needs flow to build).
    pub warmup_secs: f64,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            symbol_index: 0,
            target_lots: 1000,
            slices: 20,
            horizon_secs: 300.0,
            warmup_secs: 0.0,
        }
    }
}
