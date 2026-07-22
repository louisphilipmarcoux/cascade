"""Realized-volatility proxy construction from a trade tape."""

from __future__ import annotations

import numpy as np


def realized_vol(
    ts_ns: np.ndarray,
    prices: np.ndarray,
    bin_secs: float = 300.0,
) -> np.ndarray:
    """Realized volatility per `bin_secs` bin: sqrt(sum of 1s squared
    log-returns) within each bin. Returns the per-bin RV series."""
    ts_ns = np.asarray(ts_ns, dtype=np.int64)
    prices = np.asarray(prices, dtype=float)
    if ts_ns.size < 2:
        return np.array([])
    log_p = np.log(prices)
    t0 = ts_ns[0]
    bin_ns = int(bin_secs * 1e9)
    bin_idx = (ts_ns - t0) // bin_ns
    n_bins = int(bin_idx[-1]) + 1
    rv = np.zeros(n_bins)
    # Sum of squared successive log-returns within each bin.
    dlog = np.diff(log_p)
    # Assign each return to the bin of its right endpoint.
    for i, b in enumerate(bin_idx[1:]):
        rv[b] += dlog[i] ** 2
    rv = np.sqrt(rv)
    return rv[rv > 0]
