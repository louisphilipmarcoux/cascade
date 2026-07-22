"""Fat-tail diagnostics: the Hill tail-index estimator."""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class HillResult:
    # Tail index alpha (smaller = fatter tail; ~3 is the empirical "cubic law").
    alpha: float
    k: int
    tail_fraction: float


def hill_estimator(data: np.ndarray, tail_fraction: float = 0.05) -> HillResult:
    """Hill estimator of the tail index on the upper `tail_fraction` of |data|.

    `alpha_hat = [ (1/k) * sum_{i=1..k} ln X_(i) - ln X_(k+1) ]^-1`
    where `X_(1) >= ... >= X_(n)` are the descending order statistics of the
    magnitudes. Returns alpha (the power-law exponent), the number of order
    statistics used, and the tail fraction.
    """
    x = np.abs(np.asarray(data, dtype=float))
    x = x[np.isfinite(x) & (x > 0)]
    x = np.sort(x)[::-1]  # descending
    n = x.size
    k = max(2, int(tail_fraction * n))
    if k + 1 >= n:
        msg = "not enough data for the requested tail fraction"
        raise ValueError(msg)
    top = x[:k]
    threshold = x[k]  # X_(k+1)
    hill = np.mean(np.log(top)) - np.log(threshold)
    alpha = float(1.0 / hill) if hill > 0 else float("inf")
    return HillResult(alpha=alpha, k=k, tail_fraction=tail_fraction)


def hill_plot(data: np.ndarray, fractions: np.ndarray | None = None) -> dict[float, float]:
    """Hill alpha across a range of tail fractions (stability diagnostic)."""
    if fractions is None:
        fractions = np.linspace(0.01, 0.10, 10)
    out = {}
    for f in fractions:
        try:
            out[float(f)] = hill_estimator(data, tail_fraction=float(f)).alpha
        except ValueError:
            continue
    return out
