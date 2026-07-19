//! Performance statistics with honest inference.
//!
//! The load-bearing pieces are the **probabilistic Sharpe ratio** and the
//! **deflated Sharpe ratio** (Bailey & López de Prado 2012/2014): DSR's trial
//! count K and trial Sharpes come from the *actual* study runner — every
//! parameter combination executed against a dataset counts — never from a
//! guessed constant. Single runs report `dsr: None` with reason `"K=1"`.

pub mod normal;

use serde::{Deserialize, Serialize};

use normal::{norm_cdf, norm_inv_cdf};

/// Euler–Mascheroni constant.
const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;

/// Moments of a return series (per-period, no annualization inside).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Moments {
    pub n: usize,
    pub mean: f64,
    /// Sample std (ddof = 1).
    pub std: f64,
    pub skew: f64,
    /// Raw (non-excess) kurtosis; 3 for a Gaussian.
    pub kurtosis: f64,
}

#[must_use]
pub fn moments(returns: &[f64]) -> Option<Moments> {
    let n = returns.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mean = returns.iter().sum::<f64>() / nf;
    let m2 = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / nf;
    if m2 <= 0.0 {
        return Some(Moments {
            n,
            mean,
            std: 0.0,
            skew: 0.0,
            kurtosis: 3.0,
        });
    }
    let m3 = returns.iter().map(|r| (r - mean).powi(3)).sum::<f64>() / nf;
    let m4 = returns.iter().map(|r| (r - mean).powi(4)).sum::<f64>() / nf;
    let std = libm::sqrt(m2 * nf / (nf - 1.0));
    Some(Moments {
        n,
        mean,
        std,
        skew: m3 / m2.powf(1.5),
        kurtosis: m4 / (m2 * m2),
    })
}

/// Per-period Sharpe ratio (mean/std, ddof=1). `None` for degenerate series.
#[must_use]
pub fn sharpe(returns: &[f64]) -> Option<f64> {
    let m = moments(returns)?;
    (m.std > 0.0).then(|| m.mean / m.std)
}

/// Sortino: mean over downside deviation (vs 0 target).
#[must_use]
pub fn sortino(returns: &[f64]) -> Option<f64> {
    let m = moments(returns)?;
    let downside_sq = returns
        .iter()
        .map(|&r| if r < 0.0 { r * r } else { 0.0 })
        .sum::<f64>()
        / (m.n as f64 - 1.0);
    let dd = libm::sqrt(downside_sq);
    (dd > 0.0).then(|| m.mean / dd)
}

/// Probabilistic Sharpe Ratio: P(true SR > `sr_benchmark`) given the
/// observed series (Bailey & López de Prado 2012).
///
/// `PSR = Φ( (ŜR − SR*)·√(n−1) / √(1 − γ₃·ŜR + ((γ₄−1)/4)·ŜR²) )`
#[must_use]
pub fn probabilistic_sharpe(m: &Moments, sr_benchmark: f64) -> Option<f64> {
    if m.std <= 0.0 || m.n < 2 {
        return None;
    }
    let sr = m.mean / m.std;
    let variance_term = 1.0 - m.skew * sr + (m.kurtosis - 1.0) / 4.0 * sr * sr;
    if variance_term <= 0.0 {
        return None;
    }
    let z = (sr - sr_benchmark) * libm::sqrt(m.n as f64 - 1.0) / libm::sqrt(variance_term);
    Some(norm_cdf(z))
}

/// Expected maximum Sharpe under K independent trials with Sharpe variance
/// `var_trial_sr` (the DSR benchmark SR₀, Bailey & López de Prado 2014).
#[must_use]
pub fn expected_max_sharpe(k_trials: usize, var_trial_sr: f64) -> Option<f64> {
    if k_trials < 2 || var_trial_sr <= 0.0 {
        return None;
    }
    let k = k_trials as f64;
    Some(
        libm::sqrt(var_trial_sr)
            * ((1.0 - EULER_GAMMA) * norm_inv_cdf(1.0 - 1.0 / k)
                + EULER_GAMMA * norm_inv_cdf(1.0 - 1.0 / (k * core::f64::consts::E))),
    )
}

/// Deflated Sharpe Ratio of the *selected* (best) strategy given all trial
/// Sharpes actually run. `None` when K < 2 (reported as reason "K=1").
#[must_use]
pub fn deflated_sharpe(selected: &Moments, trial_sharpes: &[f64]) -> Option<f64> {
    let k = trial_sharpes.len();
    if k < 2 {
        return None;
    }
    let mean = trial_sharpes.iter().sum::<f64>() / k as f64;
    let var = trial_sharpes
        .iter()
        .map(|s| (s - mean).powi(2))
        .sum::<f64>()
        / (k as f64 - 1.0);
    let sr0 = expected_max_sharpe(k, var)?;
    probabilistic_sharpe(selected, sr0)
}

/// Max drawdown (fraction of peak, ≥ 0) and its duration in samples, from an
/// equity series in e8 relative to a starting equity offset.
#[must_use]
pub fn max_drawdown(equity: &[i128], base_e8: i128) -> (f64, usize) {
    let mut peak = i128::MIN;
    let mut max_dd = 0.0f64;
    let mut dd_len = 0usize;
    let mut cur_len = 0usize;
    for &e in equity {
        let level = base_e8 + e;
        if level > peak {
            peak = level;
            cur_len = 0;
        } else {
            cur_len += 1;
            if peak > 0 {
                let dd = (peak - level) as f64 / peak as f64;
                if dd > max_dd {
                    max_dd = dd;
                }
            }
            dd_len = dd_len.max(cur_len);
        }
    }
    (max_dd, dd_len)
}

/// Annualization factor for per-period SR given the period length in seconds
/// (crypto convention: 365 days).
#[must_use]
pub fn annualization_factor(period_secs: f64) -> f64 {
    libm::sqrt(365.0 * 24.0 * 3600.0 / period_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psr_reduces_to_gaussian_formula_when_normal() {
        // Gaussian moments: skew 0, kurtosis 3 → denominator = sqrt(1 + SR²/2).
        let m = Moments {
            n: 253,
            mean: 0.001,
            std: 0.01,
            skew: 0.0,
            kurtosis: 3.0,
        };
        let sr = 0.1;
        let expected_z = (sr - 0.0) * libm::sqrt(252.0) / libm::sqrt(1.0 + sr * sr / 2.0);
        let psr = probabilistic_sharpe(&m, 0.0).unwrap();
        assert!((psr - norm_cdf(expected_z)).abs() < 1e-12);
    }

    #[test]
    fn dsr_penalizes_many_trials() {
        let m = Moments {
            n: 1000,
            mean: 0.0005,
            std: 0.01,
            skew: -0.2,
            kurtosis: 4.0,
        };
        let few: Vec<f64> = vec![0.01, 0.05];
        let many: Vec<f64> = (0..100).map(|i| 0.01 + 0.001 * f64::from(i)).collect();
        let dsr_few = deflated_sharpe(&m, &few).unwrap();
        let dsr_many = deflated_sharpe(&m, &many).unwrap();
        assert!(
            dsr_many < dsr_few,
            "more trials must deflate harder: {dsr_many} !< {dsr_few}"
        );
        assert!(deflated_sharpe(&m, &[0.3]).is_none(), "K=1 has no DSR");
    }

    #[test]
    fn expected_max_sharpe_grows_with_k() {
        let a = expected_max_sharpe(10, 0.01).unwrap();
        let b = expected_max_sharpe(1000, 0.01).unwrap();
        assert!(b > a && a > 0.0);
    }

    #[test]
    fn drawdown_finds_peak_to_trough() {
        // Equity walks +10, +20, +5, +15, -10 around base 100e8.
        let base = 100 * 100_000_000i128;
        let path: Vec<i128> = [10i128, 20, 5, 15, -10]
            .iter()
            .map(|v| v * 100_000_000)
            .collect();
        let (dd, len) = max_drawdown(&path, base);
        // Peak 120, trough 90 → dd = 30/120 = 0.25, lasting to the end (3).
        assert!((dd - 0.25).abs() < 1e-12, "dd {dd}");
        assert_eq!(len, 3);
    }

    #[test]
    fn sortino_ignores_upside_vol() {
        let up_vol = [0.02, -0.01, 0.03, -0.01, 0.04, -0.01];
        let down_vol = [-0.02, 0.01, -0.03, 0.01, -0.04, 0.01];
        assert!(sortino(&up_vol).unwrap() > 0.0);
        assert!(sortino(&down_vol).unwrap() < 0.0);
    }
}
