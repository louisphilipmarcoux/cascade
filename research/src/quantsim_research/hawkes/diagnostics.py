"""Hawkes goodness-of-fit via the time-rescaling theorem (Ogata 1988).

Under a correct fit, the compensator-transformed inter-event times are iid
Exp(1). We test that with a KS test and expose the residuals for a QQ plot.
A well-specified model is not rejected; a misspecified one is.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from scipy import stats


@dataclass
class RescalingResult:
    residuals: np.ndarray
    ks_statistic: float
    ks_pvalue: float


def time_rescaling_residuals(
    events: list[np.ndarray],
    mu: np.ndarray,
    alpha: np.ndarray,
    beta: float,
    dim: int,
) -> RescalingResult:
    """Compensator-rescaled inter-arrival residuals for dimension `dim`.

    τ_k = Λ_dim(t_k) − Λ_dim(t_{k-1}), which should be iid Exp(1). For the
    exponential kernel the compensator increment has a closed form via the
    same recursion used in the likelihood.
    """
    dims = len(events)
    merged = sorted([(t, d) for d, ts in enumerate(events) for t in ts], key=lambda x: x[0])
    excitation = np.zeros((dims, dims))
    last_t = 0.0
    # Compensator of `dim` accumulates the baseline + integral of excitation.
    comp = 0.0
    comp_at_last_event = 0.0
    taus = []
    for t, d in merged:
        dt = t - last_t
        # Integral of dim's intensity over (last_t, t]:
        #   mu[dim]*dt + sum_n (excite[dim,n]/beta)*(1 - e^{-beta dt})
        comp += mu[dim] * dt
        comp += (excitation[dim].sum() / beta) * (1.0 - np.exp(-beta * dt))
        excitation *= np.exp(-beta * dt)
        last_t = t
        if d == dim:
            taus.append(comp - comp_at_last_event)
            comp_at_last_event = comp
        excitation[:, d] += alpha[:, d]

    residuals = np.array(taus)
    if residuals.size < 5:
        return RescalingResult(
            residuals=residuals, ks_statistic=float("nan"), ks_pvalue=float("nan")
        )
    ks = stats.kstest(residuals, "expon")
    return RescalingResult(
        residuals=residuals,
        ks_statistic=float(ks.statistic),
        ks_pvalue=float(ks.pvalue),
    )
