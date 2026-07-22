"""Autocorrelation diagnostics: the volatility-clustering signature.

Cont's stylized facts: raw returns show *no* linear autocorrelation beyond a
lag or two, while |returns| and returns² show slowly-decaying positive
autocorrelation (volatility clustering). This module quantifies both.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


def acf(x: np.ndarray, max_lag: int) -> np.ndarray:
    """Sample autocorrelation at lags 1..max_lag (lag 0 excluded)."""
    x = np.asarray(x, dtype=float)
    x = x[np.isfinite(x)]
    x = x - x.mean()
    var = np.dot(x, x)
    if var == 0:
        return np.zeros(max_lag)
    out = np.empty(max_lag)
    for lag in range(1, max_lag + 1):
        out[lag - 1] = np.dot(x[:-lag], x[lag:]) / var
    return out


def bartlett_band(n: int, confidence: float = 0.95) -> float:
    """Two-sided white-noise ACF confidence band (±z/√n)."""
    from scipy import stats

    z = stats.norm.ppf(0.5 + confidence / 2.0)
    return float(z / np.sqrt(n))


@dataclass(frozen=True)
class LjungBox:
    statistic: float
    p_value: float
    lags: int


def ljung_box(x: np.ndarray, lags: int = 20) -> LjungBox:
    """Ljung–Box Q test for autocorrelation up to `lags`."""
    from scipy import stats

    x = np.asarray(x, dtype=float)
    x = x[np.isfinite(x)]
    n = x.size
    rho = acf(x, lags)
    q = n * (n + 2) * np.sum(rho**2 / (n - np.arange(1, lags + 1)))
    p = float(stats.chi2.sf(q, lags))
    return LjungBox(statistic=float(q), p_value=p, lags=lags)


@dataclass(frozen=True)
class ClusteringDecay:
    # Power-law exponent eta in acf(|r|)(tau) ~ tau^-eta (slow decay ⇒ small eta).
    eta: float
    r_squared: float
    max_lag: int


def volatility_clustering_decay(returns: np.ndarray, max_lag: int = 1000) -> ClusteringDecay:
    """Fit a power law to the ACF of |returns|: the volatility-clustering
    decay exponent. Uses only positive-ACF lags in the log-log regression."""
    abs_r = np.abs(np.asarray(returns, dtype=float))
    max_lag = min(max_lag, abs_r.size // 3)
    rho = acf(abs_r, max_lag)
    lags = np.arange(1, max_lag + 1)
    mask = rho > 0
    if mask.sum() < 5:
        return ClusteringDecay(eta=float("nan"), r_squared=0.0, max_lag=max_lag)
    log_lag = np.log(lags[mask])
    log_rho = np.log(rho[mask])
    slope, intercept = np.polyfit(log_lag, log_rho, 1)
    fitted = slope * log_lag + intercept
    ss_res = np.sum((log_rho - fitted) ** 2)
    ss_tot = np.sum((log_rho - log_rho.mean()) ** 2)
    r2 = 1.0 - ss_res / ss_tot if ss_tot > 0 else 0.0
    return ClusteringDecay(eta=float(-slope), r_squared=float(r2), max_lag=max_lag)
