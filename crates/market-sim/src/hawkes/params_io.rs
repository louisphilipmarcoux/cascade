//! The shared Hawkes params TOML artifact (docs/interchange.md).
//!
//! Written by both fitters (this crate's CLI and the Python research
//! layer); the simulator reads `[baseline]`, `[kernel]`, `[marks]` only.
//! Writers refuse to emit unstable fits.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::FitResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HawkesParamsFile {
    pub schema_version: u32,
    /// `hawkes_exp_mv` (Stage 2 adds `hawkes_sumexp_mv`).
    pub model: String,
    pub dims: Vec<String>,
    pub symbol: String,
    pub exchange: String,
    pub time_unit: String,
    pub baseline: Baseline,
    pub kernel: Kernel,
    #[serde(default)]
    pub marks: Option<Marks>,
    #[serde(default)]
    pub diagnostics: Option<Diagnostics>,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    pub mu: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kernel {
    pub alpha: Vec<Vec<f64>>,
    pub beta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marks {
    pub size_dist: String,
    /// Per-dim (mu, sigma) of log size in lots.
    pub params: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostics {
    pub loglik: f64,
    pub aic: f64,
    pub branching_matrix: Vec<Vec<f64>>,
    pub spectral_radius: f64,
    pub stability_ok: bool,
    #[serde(default)]
    pub poisson_loglik: Option<f64>,
    #[serde(default)]
    pub n_events: Option<u64>,
    #[serde(default)]
    pub t_span_secs: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub created_by: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("toml: {0}")]
    Emit(#[from] toml::ser::Error),
    #[error("invalid params: {0}")]
    Invalid(String),
}

pub fn from_fit(
    fit: &FitResult,
    dims: Vec<String>,
    symbol: &str,
    exchange: &str,
    source: &str,
) -> HawkesParamsFile {
    HawkesParamsFile {
        schema_version: 1,
        model: "hawkes_exp_mv".to_string(),
        dims,
        symbol: symbol.to_string(),
        exchange: exchange.to_string(),
        time_unit: "seconds".to_string(),
        baseline: Baseline { mu: fit.mu.clone() },
        kernel: Kernel {
            alpha: fit.alpha.clone(),
            beta: fit.beta,
        },
        marks: None,
        diagnostics: Some(Diagnostics {
            loglik: fit.log_likelihood,
            aic: fit.aic,
            branching_matrix: fit.branching_matrix.clone(),
            spectral_radius: fit.spectral_radius,
            stability_ok: fit.spectral_radius < 1.0,
            poisson_loglik: Some(fit.poisson_log_likelihood),
            n_events: Some(fit.n_events as u64),
            t_span_secs: Some(fit.t_span),
        }),
        provenance: Some(Provenance {
            source: source.to_string(),
            created_by: format!("quant-sim {} (rust fitter)", env!("CARGO_PKG_VERSION")),
        }),
    }
}

/// Write the artifact; refuses unstable parameter sets.
pub fn write_params(path: &Path, params: &HawkesParamsFile) -> Result<(), ParamsError> {
    let branching: Vec<Vec<f64>> = params
        .kernel
        .alpha
        .iter()
        .map(|row| row.iter().map(|a| a / params.kernel.beta).collect())
        .collect();
    let radius = super::spectral_radius(&branching);
    if radius >= 1.0 {
        return Err(ParamsError::Invalid(format!(
            "refusing to write unstable fit (spectral radius {radius:.4})"
        )));
    }
    let text = toml::to_string_pretty(params)?;
    std::fs::write(path, text.replace("\r\n", "\n"))?;
    Ok(())
}

pub fn read_params(path: &Path) -> Result<HawkesParamsFile, ParamsError> {
    let text = std::fs::read_to_string(path)?;
    let params: HawkesParamsFile = toml::from_str(&text)?;
    if params.model != "hawkes_exp_mv" {
        return Err(ParamsError::Invalid(format!(
            "unsupported model {:?} (this build simulates hawkes_exp_mv)",
            params.model
        )));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_unstable() {
        let fit = FitResult {
            mu: vec![0.4, 0.4],
            alpha: vec![vec![0.6, 0.15], vec![0.15, 0.6]],
            beta: 1.5,
            log_likelihood: -100.0,
            aic: 214.0,
            branching_matrix: vec![vec![0.4, 0.1], vec![0.1, 0.4]],
            spectral_radius: 0.5,
            n_events: 1000,
            t_span: 3600.0,
            poisson_log_likelihood: -150.0,
        };
        let params = from_fit(
            &fit,
            vec!["buy".into(), "sell".into()],
            "BTCUSDT",
            "binance-um",
            "test",
        );
        let dir = std::env::temp_dir().join("quant-sim-params-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fit.toml");
        write_params(&path, &params).unwrap();
        let back = read_params(&path).unwrap();
        assert_eq!(back.baseline.mu, params.baseline.mu);
        assert_eq!(back.kernel.alpha, params.kernel.alpha);

        let mut unstable = params.clone();
        unstable.kernel.beta = 0.1; // ρ becomes >> 1
        assert!(write_params(&path, &unstable).is_err());
    }
}
