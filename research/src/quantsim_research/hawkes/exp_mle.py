"""2-dim exponential-kernel Hawkes MLE (scipy), mirroring the Rust fitter.

Same recursive O(N) log-likelihood (Ozaki), same log-parameter transform,
same shared-β model. The optimizer is scipy L-BFGS-B with multi-start; the
Rust side uses a hand-rolled Nelder-Mead. A cross-implementation agreement
test holds the two to matching log-likelihoods.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from scipy.optimize import minimize


@dataclass
class HawkesFit:
    mu: np.ndarray  # (2,)
    alpha: np.ndarray  # (2, 2)
    beta: float
    log_likelihood: float
    branching_matrix: np.ndarray
    spectral_radius: float
    poisson_log_likelihood: float
    n_events: int
    t_span: float


def _spread_ties(times: np.ndarray) -> tuple[np.ndarray, float]:
    """Deterministically spread tied timestamps (matches the Rust fitter's
    approach for millisecond-resolution real data). Returns spread times and
    the minimum spacing introduced."""
    times = np.sort(times)
    n = times.size
    if n < 2:
        return times, 1.0
    span = times[-1] - times[0]
    default_gap = max(span / n, 1e-9)
    out = times.astype(float).copy()
    min_spacing = np.inf
    i = 0
    while i < n:
        j = i + 1
        while j < n and out[j] <= out[i]:
            j += 1
        run = j - i
        if run > 1:
            nxt = out[j] if j < n else out[i] + default_gap
            step = (nxt - out[i]) / run
            for k in range(run):
                out[i + k] = out[i] + step * k
            min_spacing = min(min_spacing, max(step, 1e-12))
        elif j < n:
            min_spacing = min(min_spacing, max(out[j] - out[i], 1e-12))
        i = j
    if not np.isfinite(min_spacing):
        min_spacing = default_gap
    return out, float(min_spacing)


def _neg_log_likelihood(
    theta: np.ndarray, events: list[np.ndarray], t_end: float, beta_max: float
) -> float:
    dims = len(events)
    mu = np.exp(theta[:dims])
    alpha = np.exp(theta[dims : dims + dims * dims]).reshape(dims, dims)
    beta = np.exp(theta[-1])
    if beta > beta_max:
        return 1e12 + beta
    branching = alpha / beta
    rho = np.max(np.abs(np.linalg.eigvals(branching)))
    if rho >= 0.999:
        return 1e12 + rho * 1e6
    penalty = 1e4 * (rho - 0.95) ** 2 if rho > 0.95 else 0.0

    # Merge into one sorted (time, dim) stream.
    merged = sorted([(t, d) for d, ts in enumerate(events) for t in ts], key=lambda x: x[0])
    excitation = np.zeros((dims, dims))
    last_t = 0.0
    log_term = 0.0
    compensator = 0.0
    for t, dim in merged:
        decay = np.exp(-beta * (t - last_t))
        excitation *= decay
        last_t = t
        intensity = mu[dim] + excitation[dim].sum()
        if intensity <= 0:
            return 1e12
        log_term += np.log(intensity)
        tail = 1.0 - np.exp(-beta * (t_end - t))
        excitation[:, dim] += alpha[:, dim]
        compensator += (alpha[:, dim] / beta * tail).sum()
    baseline = mu.sum() * t_end
    return -(log_term - baseline - compensator) + penalty


def fit_exp_hawkes(events: list[np.ndarray], t_end: float) -> HawkesFit:
    """Fit a 2-dim exponential Hawkes by MLE. `events` is a list of per-dim
    event-time arrays (seconds)."""
    dims = len(events)
    n_events = sum(len(e) for e in events)
    if n_events < 10 * dims:
        msg = f"too few events ({n_events})"
        raise ValueError(msg)

    # Spread ties across all dims jointly, then re-split by original dim tag.
    tagged = sorted([(t, d) for d, ts in enumerate(events) for t in ts], key=lambda x: x[0])
    times = np.array([t for t, _ in tagged])
    spread, min_spacing = _spread_ties(times)
    spread_events = [[] for _ in range(dims)]
    for (_, d), t in zip(tagged, spread, strict=True):
        spread_events[d].append(t)
    spread_events = [np.array(e) for e in spread_events]
    beta_max = 20.0 / min_spacing

    mean_rate = n_events / t_end / dims
    best = None
    for share, beta0 in [(0.2, 0.5), (0.2, 5.0), (0.6, 1.0), (0.6, 20.0)]:
        mu0 = max(mean_rate * (1 - share), 1e-6)
        alpha0 = max(share * beta0 / dims, 1e-6)
        x0 = np.concatenate(
            [
                np.full(dims, np.log(mu0)),
                np.full(dims * dims, np.log(alpha0)),
                [np.log(beta0)],
            ]
        )
        res = minimize(
            _neg_log_likelihood,
            x0,
            args=(spread_events, t_end, beta_max),
            method="L-BFGS-B",
        )
        if best is None or res.fun < best.fun:
            best = res

    theta = best.x
    mu = np.exp(theta[:dims])
    alpha = np.exp(theta[dims : dims + dims * dims]).reshape(dims, dims)
    beta = float(np.exp(theta[-1]))
    branching = alpha / beta
    rho = float(np.max(np.abs(np.linalg.eigvals(branching))))
    poisson_ll = sum(
        len(e) * np.log(len(e) / t_end) - len(e) if len(e) > 0 else 0.0 for e in events
    )
    return HawkesFit(
        mu=mu,
        alpha=alpha,
        beta=beta,
        log_likelihood=float(-best.fun),
        branching_matrix=branching,
        spectral_radius=rho,
        poisson_log_likelihood=float(poisson_ll),
        n_events=n_events,
        t_span=t_end,
    )
