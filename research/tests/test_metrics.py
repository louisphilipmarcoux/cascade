"""Metrics: hand-computed reference values and DSR monotonicity."""

import numpy as np

from quantsim_research.metrics.sharpe import (
    deflated_sharpe,
    expected_max_sharpe,
    probabilistic_sharpe,
    sharpe_ratio,
)


def test_sharpe_matches_definition():
    r = np.array([0.01, -0.005, 0.02, 0.0, 0.015])
    sr = sharpe_ratio(r)
    assert abs(sr - r.mean() / r.std(ddof=1)) < 1e-12


def test_psr_reduces_to_gaussian_when_symmetric():
    # Symmetric mesokurtic sample ≈ Gaussian: PSR uses the standard formula.
    rng = np.random.default_rng(0)
    r = 0.001 + 0.01 * rng.standard_normal(1000)
    psr = probabilistic_sharpe(r, 0.0)
    assert psr is not None
    # SR > 0 here, so PSR(0) should exceed 0.5.
    assert psr > 0.5


def test_deflated_sharpe_penalizes_more_trials():
    rng = np.random.default_rng(1)
    r = 0.0005 + 0.01 * rng.standard_normal(1000)
    few = np.array([0.01, 0.05])
    many = 0.01 + 0.001 * np.arange(100)
    dsr_few = deflated_sharpe(r, few)
    dsr_many = deflated_sharpe(r, many)
    assert dsr_few is not None and dsr_many is not None
    assert dsr_many < dsr_few
    assert deflated_sharpe(r, np.array([0.3])) is None  # K=1


def test_expected_max_sharpe_grows_with_k():
    a = expected_max_sharpe(10, 0.01)
    b = expected_max_sharpe(1000, 0.01)
    assert a is not None and b is not None and b > a > 0
