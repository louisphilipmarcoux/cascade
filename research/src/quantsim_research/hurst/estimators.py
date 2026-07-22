"""Hurst-exponent estimators: DFA, R/S, and the GJR smoothness regression.

All three are calibrated in tests against synthetic fractional Gaussian
noise / fractional Brownian motion with known H.
"""

from __future__ import annotations

import numpy as np


def _log_spaced(lo: int, hi: int, count: int) -> np.ndarray:
    return np.unique(np.round(np.geomspace(lo, hi, count)).astype(int))


def dfa_hurst(series: np.ndarray, min_window: int = 8, n_scales: int = 20) -> float:
    """Detrended Fluctuation Analysis (order 1). Returns the Hurst exponent
    from the slope of log F(n) vs log n."""
    x = np.asarray(series, dtype=float)
    x = x[np.isfinite(x)]
    n = x.size
    if n < 4 * min_window:
        return float("nan")
    profile = np.cumsum(x - x.mean())
    scales = _log_spaced(min_window, n // 4, n_scales)
    fluct = []
    for s in scales:
        n_segments = n // s
        if n_segments < 1:
            continue
        rms = []
        for seg in range(n_segments):
            piece = profile[seg * s : (seg + 1) * s]
            t = np.arange(s)
            coef = np.polyfit(t, piece, 1)
            trend = np.polyval(coef, t)
            rms.append(np.sqrt(np.mean((piece - trend) ** 2)))
        fluct.append(np.sqrt(np.mean(np.square(rms))))
    fluct = np.array(fluct)
    scales = scales[: len(fluct)]
    mask = fluct > 0
    if mask.sum() < 3:
        return float("nan")
    slope, _ = np.polyfit(np.log(scales[mask]), np.log(fluct[mask]), 1)
    return float(slope)


def rescaled_range_hurst(series: np.ndarray, min_window: int = 8, n_scales: int = 20) -> float:
    """Classical rescaled-range (R/S) Hurst estimator. Lo's small-sample bias
    caveat applies; kept as a secondary/legacy estimator."""
    x = np.asarray(series, dtype=float)
    x = x[np.isfinite(x)]
    n = x.size
    if n < 4 * min_window:
        return float("nan")
    scales = _log_spaced(min_window, n // 2, n_scales)
    rs = []
    used = []
    for s in scales:
        n_segments = n // s
        if n_segments < 1:
            continue
        ratios = []
        for seg in range(n_segments):
            piece = x[seg * s : (seg + 1) * s]
            mean = piece.mean()
            dev = np.cumsum(piece - mean)
            r = dev.max() - dev.min()
            sd = piece.std()
            if sd > 0:
                ratios.append(r / sd)
        if ratios:
            rs.append(np.mean(ratios))
            used.append(s)
    if len(rs) < 3:
        return float("nan")
    slope, _ = np.polyfit(np.log(used), np.log(rs), 1)
    return float(slope)


def gjr_smoothness_hurst(
    log_vol: np.ndarray,
    qs: tuple[float, ...] = (0.5, 1.0, 1.5, 2.0, 3.0),
    max_lag: int = 50,
) -> float:
    """Gatheral–Jaisson–Rosenbaum smoothness regression.

    For each moment order q, `m(q, Δ) = <|log σ_{t+Δ} − log σ_t|^q>` scales
    like `Δ^{ζ_q}`; H is the slope of ζ_q vs q. Rough volatility ⇒ H < 0.5.
    """
    lv = np.asarray(log_vol, dtype=float)
    lv = lv[np.isfinite(lv)]
    n = lv.size
    if n < 4 * max_lag:
        return float("nan")
    lags = _log_spaced(1, max_lag, 12)
    zetas = []
    valid_qs = []
    for q in qs:
        log_m = []
        log_d = []
        for d in lags:
            diffs = np.abs(lv[d:] - lv[:-d]) ** q
            m = diffs.mean()
            if m > 0:
                log_m.append(np.log(m))
                log_d.append(np.log(d))
        if len(log_m) >= 3:
            slope, _ = np.polyfit(log_d, log_m, 1)
            zetas.append(slope)
            valid_qs.append(q)
    if len(zetas) < 2:
        return float("nan")
    # H = slope of ζ_q vs q (through the moment orders).
    h, _ = np.polyfit(valid_qs, zetas, 1)
    return float(h)
