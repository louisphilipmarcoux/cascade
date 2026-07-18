//! Event-driven backtester.
//!
//! Lands at M5: `Strategy` trait with a delayed book view (structural
//! look-ahead defense), broker, i128 cash accounting with a venue fee ledger,
//! metrics (Sharpe/Sortino/PSR/DSR with a real trials ledger), walk-forward
//! utilities, and JSON run reports.
