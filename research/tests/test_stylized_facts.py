"""Stylized-facts battery: sanity on synthetic series with known properties."""

import numpy as np

from quantsim_research.stylized_facts.autocorr import (
    acf,
    ljung_box,
    volatility_clustering_decay,
)
from quantsim_research.stylized_facts.returns import moments
from quantsim_research.stylized_facts.tails import hill_estimator


def garch_like(n: int, seed: int = 0) -> np.ndarray:
    """A GARCH(1,1) series with Student-t innovations: volatility clusters
    (the ARCH feedback) and genuinely fat tails (heavy-tailed shocks)."""
    rng = np.random.default_rng(seed)
    omega, a, b = 1e-6, 0.12, 0.86
    var = omega / (1 - a - b)
    out = np.empty(n)
    df = 5.0
    scale = np.sqrt((df - 2) / df)  # unit-variance Student-t
    for i in range(n):
        eps = rng.standard_t(df) * scale
        out[i] = np.sqrt(var) * eps
        var = omega + a * out[i] ** 2 + b * var
    return out


def test_iid_gaussian_has_no_clustering():
    rng = np.random.default_rng(1)
    r = rng.standard_normal(20_000)
    # |r| ACF should be within noise for iid data.
    rho = acf(np.abs(r), 50)
    assert np.mean(np.abs(rho)) < 0.03
    m = moments(r)
    assert abs(m.excess_kurtosis) < 0.3


def test_garch_shows_fat_tails_and_clustering():
    r = garch_like(30_000, seed=2)
    m = moments(r)
    # Fat tails: strongly positive excess kurtosis.
    assert m.excess_kurtosis > 1.0
    # Volatility clustering: |r| autocorrelation strongly positive at lag 1.
    rho = acf(np.abs(r), 50)
    assert rho[0] > 0.05
    # Ljung-Box on |r| rejects the null decisively.
    lb = ljung_box(np.abs(r), lags=20)
    assert lb.p_value < 1e-6
    # Ljung-Box on raw r does NOT (returns are ~uncorrelated).
    lb_raw = ljung_box(r, lags=20)
    assert lb_raw.p_value > lb.p_value


def test_hill_recovers_pareto_tail():
    # Pareto(alpha=3) tail; Hill should recover ~3.
    rng = np.random.default_rng(3)
    alpha_true = 3.0
    x = (1.0 - rng.random(50_000)) ** (-1.0 / alpha_true)
    est = hill_estimator(x, tail_fraction=0.05)
    assert abs(est.alpha - alpha_true) < 0.4


def test_clustering_decay_is_slow_for_garch():
    r = garch_like(40_000, seed=5)
    decay = volatility_clustering_decay(r, max_lag=500)
    # Slow decay ⇒ small positive exponent.
    assert 0.0 < decay.eta < 1.5
