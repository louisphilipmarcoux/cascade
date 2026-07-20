//! `quant-sim fit`: model fitting from interchange data.

use std::path::Path;

use market_sim::data::{read_trades_csv, trades_to_hawkes_events};
use market_sim::hawkes::{fit_exp_mle, params_io};
use strategies::fit_ou;

#[derive(Debug, thiserror::Error)]
pub enum FitError {
    #[error("data: {0}")]
    Data(#[from] market_sim::data::DataError),
    #[error("fit: {0}")]
    Fit(String),
    #[error("params: {0}")]
    Params(#[from] market_sim::hawkes::params_io::ParamsError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Fit a 2-dim (buy/sell aggressor) exponential Hawkes to a trades CSV and
/// write the shared params TOML.
pub fn hawkes(trades_csv: &Path, out: &Path, symbol: &str, exchange: &str) -> Result<(), FitError> {
    let rows = read_trades_csv(trades_csv)?;
    let (events, t_span) = trades_to_hawkes_events(&rows);
    eprintln!(
        "fitting 2-dim exp Hawkes: {} buys, {} sells over {:.1}s",
        events[0].len(),
        events[1].len(),
        t_span
    );
    let fit = fit_exp_mle(&events, t_span).map_err(FitError::Fit)?;
    eprintln!(
        "  mu = [{:.4}, {:.4}] /s   beta = {:.3} /s   branching ρ(G) = {:.3}",
        fit.mu[0], fit.mu[1], fit.beta, fit.spectral_radius
    );
    eprintln!(
        "  logL = {:.1}  (Poisson {:.1}, ΔlogL = {:.1})  AIC = {:.1}",
        fit.log_likelihood,
        fit.poisson_log_likelihood,
        fit.log_likelihood - fit.poisson_log_likelihood,
        fit.aic
    );
    let params = params_io::from_fit(
        &fit,
        vec!["buy".into(), "sell".into()],
        symbol,
        exchange,
        &trades_csv.display().to_string(),
    );
    params_io::write_params(out, &params)?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Fit an OU process to a `(ts_ns, value)` CSV; JSON result on stdout.
pub fn ou(series_csv: &Path) -> Result<(), FitError> {
    let mut reader =
        csv::Reader::from_path(series_csv).map_err(|e| FitError::Fit(format!("csv: {e}")))?;
    let mut ts: Vec<i64> = Vec::new();
    let mut xs: Vec<f64> = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| FitError::Fit(format!("csv: {e}")))?;
        ts.push(
            record
                .get(0)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| FitError::Fit("bad ts column".into()))?,
        );
        xs.push(
            record
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| FitError::Fit("bad value column".into()))?,
        );
    }
    if ts.len() < 3 {
        return Err(FitError::Fit("need at least 3 rows".into()));
    }
    // Median spacing in seconds (robust to occasional gaps).
    let mut gaps: Vec<i64> = ts.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let dt = gaps[gaps.len() / 2] as f64 / 1e9;
    let fit = fit_ou(&xs, dt).map_err(FitError::Fit)?;
    println!("{}", serde_json::to_string_pretty(&fit)?);
    Ok(())
}
