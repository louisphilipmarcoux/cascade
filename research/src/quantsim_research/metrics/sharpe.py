"""Sharpe, probabilistic Sharpe and deflated Sharpe (Bailey & López de Prado).

Formulas match `crates/backtester/src/metrics/mod.rs` exactly so the two
implementations cross-check.
"""

from __future__ import annotations

import numpy as np
from scipy import stats

EULER_GAMMA = 0.5772156649015329


def sharpe_ratio(returns: np.ndarray) -> float | None:
    """Per-period Sharpe (mean/std, ddof=1)."""
    r = np.asarray(returns, dtype=float)
    r = r[np.isfinite(r)]
    if r.size < 2:
        return None
    std = r.std(ddof=1)
    if std == 0:
        return None
    return float(r.mean() / std)


def sortino_ratio(returns: np.ndarray) -> float | None:
    """Per-period Sortino: mean over downside deviation (vs 0 target)."""
    r = np.asarray(returns, dtype=float)
    r = r[np.isfinite(r)]
    n = r.size
    if n < 2:
        return None
    downside = np.where(r < 0, r, 0.0)
    dd = np.sqrt(np.sum(downside**2) / (n - 1))
    if dd == 0:
        return None
    return float(r.mean() / dd)


def _skew_kurt(returns: np.ndarray) -> tuple[float, float]:
    """Skewness and *raw* (non-excess) kurtosis, matching the Rust code."""
    r = np.asarray(returns, dtype=float)
    centered = r - r.mean()
    m2 = np.mean(centered**2)
    m3 = np.mean(centered**3)
    m4 = np.mean(centered**4)
    skew = m3 / m2**1.5
    kurt = m4 / m2**2  # raw kurtosis (3 for Gaussian)
    return float(skew), float(kurt)


def probabilistic_sharpe(returns: np.ndarray, sr_benchmark: float = 0.0) -> float | None:
    """PSR(SR*) = Φ( (ŜR − SR*)·√(n−1) / √(1 − γ₃·ŜR + ((γ₄−1)/4)·ŜR²) )."""
    r = np.asarray(returns, dtype=float)
    r = r[np.isfinite(r)]
    n = r.size
    if n < 2:
        return None
    sr = sharpe_ratio(r)
    if sr is None:
        return None
    skew, kurt = _skew_kurt(r)
    variance_term = 1.0 - skew * sr + (kurt - 1.0) / 4.0 * sr * sr
    if variance_term <= 0:
        return None
    z = (sr - sr_benchmark) * np.sqrt(n - 1) / np.sqrt(variance_term)
    return float(stats.norm.cdf(z))


def expected_max_sharpe(k_trials: int, var_trial_sr: float) -> float | None:
    """SR₀ = √V[SR_k]·((1−γ)Φ⁻¹(1−1/K) + γΦ⁻¹(1−1/(Ke)))."""
    if k_trials < 2 or var_trial_sr <= 0:
        return None
    k = float(k_trials)
    return float(
        np.sqrt(var_trial_sr)
        * (
            (1.0 - EULER_GAMMA) * stats.norm.ppf(1.0 - 1.0 / k)
            + EULER_GAMMA * stats.norm.ppf(1.0 - 1.0 / (k * np.e))
        )
    )


def deflated_sharpe(returns: np.ndarray, trial_sharpes: np.ndarray) -> float | None:
    """DSR = PSR(SR₀) where SR₀ is the expected max Sharpe over K trials."""
    trials = np.asarray(trial_sharpes, dtype=float)
    if trials.size < 2:
        return None
    var = trials.var(ddof=1)
    sr0 = expected_max_sharpe(trials.size, var)
    if sr0 is None:
        return None
    return probabilistic_sharpe(returns, sr0)
