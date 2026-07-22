"""Hurst / roughness estimation (spec Section 12 cross-check).

Estimates the Hurst exponent of a realized-volatility proxy three ways
(DFA, R/S, and the Gatheral–Jaisson–Rosenbaum smoothness regression), on
both real data and simulator output. The preregistered expectation: real
crypto volatility is *rough* (H ≈ 0.05–0.2) while the v1 exponential-kernel
simulator comes out H ≈ 0.5 (not rough). Confirming exactly that gap is the
checkable prediction of the Jusselin–Rosenbaum theory and the honest
headline of the section — the power-law-kernel phase (Stage 2) then tries to
close it.
"""

from quantsim_research.hurst.estimators import (
    dfa_hurst,
    gjr_smoothness_hurst,
    rescaled_range_hurst,
)
from quantsim_research.hurst.vol_proxy import realized_vol

__all__ = [
    "dfa_hurst",
    "gjr_smoothness_hurst",
    "realized_vol",
    "rescaled_range_hurst",
]
