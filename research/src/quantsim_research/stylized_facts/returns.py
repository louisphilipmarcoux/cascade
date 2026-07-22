"""Return construction and moments."""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


def prices_to_log_returns(prices: np.ndarray) -> np.ndarray:
    """Log-returns of a strictly-positive price series."""
    prices = np.asarray(prices, dtype=float)
    if np.any(prices <= 0):
        msg = "prices must be strictly positive"
        raise ValueError(msg)
    return np.diff(np.log(prices))


def log_returns(prices: np.ndarray, sample_every: int = 1) -> np.ndarray:
    """Log-returns sampled every `sample_every` observations (aggregation)."""
    prices = np.asarray(prices, dtype=float)
    if sample_every > 1:
        prices = prices[::sample_every]
    return prices_to_log_returns(prices)


@dataclass(frozen=True)
class Moments:
    n: int
    mean: float
    std: float
    skewness: float
    # Excess kurtosis (0 for a Gaussian).
    excess_kurtosis: float

    def jarque_bera(self) -> tuple[float, float]:
        """Jarque–Bera statistic and its χ²(2) p-value."""
        from scipy import stats

        jb = self.n / 6.0 * (self.skewness**2 + self.excess_kurtosis**2 / 4.0)
        p = float(stats.chi2.sf(jb, 2))
        return float(jb), p


def moments(returns: np.ndarray) -> Moments:
    """Sample moments; excess kurtosis (Fisher)."""
    r = np.asarray(returns, dtype=float)
    r = r[np.isfinite(r)]
    n = r.size
    if n < 2:
        msg = "need at least 2 returns"
        raise ValueError(msg)
    mean = float(r.mean())
    std = float(r.std(ddof=1))
    if std == 0:
        return Moments(n=n, mean=mean, std=0.0, skewness=0.0, excess_kurtosis=0.0)
    centered = r - mean
    m2 = np.mean(centered**2)
    m3 = np.mean(centered**3)
    m4 = np.mean(centered**4)
    skew = float(m3 / m2**1.5)
    kurt = float(m4 / m2**2 - 3.0)
    return Moments(n=n, mean=mean, std=std, skewness=skew, excess_kurtosis=kurt)


def excess_kurtosis_by_scale(prices: np.ndarray, scales: list[int]) -> dict[int, float]:
    """Excess kurtosis at each aggregation scale (aggregational gaussianity:
    kurtosis should decline toward 0 as the scale grows)."""
    out = {}
    for scale in scales:
        r = log_returns(prices, sample_every=scale)
        if r.size >= 4:
            out[scale] = moments(r).excess_kurtosis
    return out
