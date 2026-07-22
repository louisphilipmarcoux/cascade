"""Hawkes: MLE parameter recovery and time-rescaling GOF."""

import numpy as np
import pytest

from quantsim_research.hawkes.diagnostics import time_rescaling_residuals
from quantsim_research.hawkes.exp_mle import fit_exp_hawkes


def simulate(mu, alpha, beta, t_end, seed=0):
    """Ogata thinning for a 2-dim exponential Hawkes (test generator)."""
    rng = np.random.default_rng(seed)
    dims = len(mu)
    events = [[] for _ in range(dims)]
    excitation = np.zeros((dims, dims))
    last_t = 0.0
    t = 0.0
    while True:
        bound = (mu + excitation.sum(axis=1)).sum()
        if bound <= 0:
            break
        t += rng.exponential(1.0 / bound)
        if t >= t_end:
            break
        excitation *= np.exp(-beta * (t - last_t))
        last_t = t
        lam = mu + excitation.sum(axis=1)
        if rng.random() * bound <= lam.sum():
            # Choose dim proportionally.
            pick = rng.random() * lam.sum()
            dim = 0
            for d in range(dims):
                if pick < lam[d]:
                    dim = d
                    break
                pick -= lam[d]
            events[dim].append(t)
            excitation[:, dim] += alpha[:, dim]
    return [np.array(e) for e in events]


@pytest.mark.slow
def test_mle_recovers_parameters():
    mu = np.array([0.4, 0.4])
    alpha = np.array([[0.6, 0.15], [0.15, 0.6]])
    beta = 1.5  # branching rho = 0.5
    events = simulate(mu, alpha, beta, t_end=30_000.0, seed=1234)
    n = sum(len(e) for e in events)
    assert n > 20_000
    fit = fit_exp_hawkes(events, 30_000.0)
    assert fit.log_likelihood > fit.poisson_log_likelihood + 100
    assert abs(fit.spectral_radius - 0.5) < 0.1
    assert abs(fit.beta - beta) / beta < 0.25
    for m in range(2):
        assert abs(fit.mu[m] - mu[m]) / mu[m] < 0.2


def test_time_rescaling_passes_for_true_model():
    mu = np.array([0.5, 0.5])
    alpha = np.array([[0.4, 0.1], [0.1, 0.4]])
    beta = 1.0
    events = simulate(mu, alpha, beta, t_end=10_000.0, seed=99)
    # Residuals under the TRUE parameters should look Exp(1) (not rejected).
    res = time_rescaling_residuals(events, mu, alpha, beta, dim=0)
    assert res.ks_pvalue > 0.01
    # Mean of Exp(1) residuals ≈ 1.
    assert abs(res.residuals.mean() - 1.0) < 0.15


def test_time_rescaling_rejects_wrong_model():
    mu = np.array([0.5, 0.5])
    alpha = np.array([[0.4, 0.1], [0.1, 0.4]])
    beta = 1.0
    events = simulate(mu, alpha, beta, t_end=10_000.0, seed=42)
    # Fit with NO excitation (pure Poisson) — the clustering shows up as
    # non-Exp(1) residuals → KS rejects.
    wrong_alpha = np.zeros((2, 2))
    res = time_rescaling_residuals(events, mu, wrong_alpha, beta, dim=0)
    assert res.ks_pvalue < 0.05
