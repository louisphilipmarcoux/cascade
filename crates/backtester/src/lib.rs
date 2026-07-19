//! Event-driven backtester with honest statistics.
//!
//! - [`strategy`]: the `Strategy` trait and its latency-honest adapter — a
//!   strategy's book view is built **only** from records it received over
//!   its delayed feed (structural look-ahead defense);
//! - [`account`]: exact `i128` accounting where the identity
//!   `equity = realized + unrealized − fees` holds by construction (P19);
//! - [`fees`]: the single adverse-rounding fee rule (P20);
//! - [`metrics`]: Sharpe/Sortino/PSR/**DSR** with in-tree normal CDF — DSR's
//!   trial count comes from real study runs, never a guess;
//! - [`runner`]: folds the authoritative event log into reports;
//! - [`report`]: the versioned JSON artifacts shared with the research layer.

pub mod account;
pub mod fees;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod strategy;

pub use account::Account;
pub use fees::FeeModel;
pub use report::{RunReport, StrategyReport, StudyReport};
pub use runner::{ReportSpec, build_report, build_study};
pub use strategy::{OpenOrder, OwnFill, Strategy, StrategyActor, TradeApi};
