//! Interchange data readers (docs/interchange.md): scaled-i64 CSV, LF, UTC
//! epoch nanoseconds — the deterministic cross-boundary format.

use std::path::Path;

use serde::Deserialize;

/// One row of the `trades` interchange schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct TradeRow {
    pub ts_ns: i64,
    /// Aggressor: +1 buy, −1 sell.
    pub side: i8,
    pub price_e8: i64,
    pub qty_e8: i64,
    pub agg_trade_id: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("csv: {0}")]
    Csv(#[from] csv::Error),
    #[error("invalid data: {0}")]
    Invalid(String),
}

/// Read and validate a trades interchange CSV: header, monotonic non-negative
/// timestamps, sane sides/quantities.
pub fn read_trades_csv(path: &Path) -> Result<Vec<TradeRow>, DataError> {
    let mut reader = csv::Reader::from_path(path)?;
    let mut rows: Vec<TradeRow> = Vec::new();
    for (line, record) in reader.deserialize::<TradeRow>().enumerate() {
        let row = record?;
        if row.side != 1 && row.side != -1 {
            return Err(DataError::Invalid(format!(
                "row {line}: side {} not in {{-1, 1}}",
                row.side
            )));
        }
        if row.qty_e8 <= 0 || row.price_e8 <= 0 || row.ts_ns < 0 {
            return Err(DataError::Invalid(format!(
                "row {line}: non-positive field"
            )));
        }
        if let Some(previous) = rows.last()
            && row.ts_ns < previous.ts_ns
        {
            return Err(DataError::Invalid(format!(
                "row {line}: timestamps not monotonic ({} < {})",
                row.ts_ns, previous.ts_ns
            )));
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(DataError::Invalid("empty trades file".into()));
    }
    Ok(rows)
}

/// Per-dimension event times in seconds (buy = dim 0, sell = dim 1) from a
/// trade tape — the Hawkes fitting input. Times are offset to start at 0.
#[must_use]
pub fn trades_to_hawkes_events(rows: &[TradeRow]) -> (Vec<Vec<f64>>, f64) {
    let t0 = rows.first().map_or(0, |r| r.ts_ns);
    let t_end = rows.last().map_or(0, |r| r.ts_ns);
    let mut events = vec![Vec::new(), Vec::new()];
    for row in rows {
        let t = (row.ts_ns - t0) as f64 / 1e9;
        let dim = usize::from(row.side < 0);
        events[dim].push(t);
    }
    (events, ((t_end - t0) as f64 / 1e9).max(f64::MIN_POSITIVE))
}
