"""Hurst estimators calibrated on synthetic fBm/fGn with known H."""

import numpy as np
import pytest

from quantsim_research.hurst.estimators import dfa_hurst, rescaled_range_hurst


def fbm(n: int, hurst: float, seed: int = 0) -> np.ndarray:
    """Fractional Brownian motion via the Hosking / Cholesky-free spectral
    method (Davies-Harte) — exact for the given H."""
    rng = np.random.default_rng(seed)

    # Davies–Harte circulant embedding of fGn covariance.
    def gamma(k: int) -> float:
        return 0.5 * (
            abs(k - 1) ** (2 * hurst) - 2 * abs(k) ** (2 * hurst) + abs(k + 1) ** (2 * hurst)
        )

    g = np.array([gamma(k) for k in range(n)])
    r = np.concatenate([g, g[-2:0:-1]])
    lam = np.fft.fft(r).real
    lam[lam < 0] = 0.0
    m = r.size
    w = rng.standard_normal(m) + 1j * rng.standard_normal(m)
    fgn = np.fft.fft(np.sqrt(lam / (2 * m)) * w).real[:n]
    return np.cumsum(fgn)


@pytest.mark.parametrize("h_true", [0.3, 0.5, 0.7])
def test_dfa_recovers_hurst(h_true):
    # DFA on fractional Gaussian noise (the fBm increments) estimates H.
    fgn = np.diff(fbm(8192, h_true, seed=int(h_true * 100)))
    h_est = dfa_hurst(fgn)
    assert abs(h_est - h_true) < 0.12, f"DFA H={h_est:.3f} vs {h_true}"


def test_rs_recovers_hurst_roughly():
    path = fbm(8192, 0.7, seed=7)
    fgn = np.diff(path)
    h = rescaled_range_hurst(fgn)
    # R/S is biased in small samples (Lo); wide tolerance.
    assert 0.5 < h < 0.95
