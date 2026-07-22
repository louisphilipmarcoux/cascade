"""Stylized-facts validation battery (Cont 2001 and extensions).

Every statistic is computed identically on real data and on simulator
output, so the headline deliverable — a real-vs-sim comparison table — is
symmetric. The battery deliberately includes statistics the v1
exponential-kernel Hawkes simulator is *expected to miss* (volatility
roughness, the Zumbach/time-reversal asymmetry), which is the calibrated-
humility result, not a defect of the analysis.
"""

from quantsim_research.stylized_facts.autocorr import (
    acf,
    ljung_box,
    volatility_clustering_decay,
)
from quantsim_research.stylized_facts.orderflow import (
    trade_sign_autocorr,
    zumbach_asymmetry,
)
from quantsim_research.stylized_facts.returns import log_returns, moments
from quantsim_research.stylized_facts.tails import hill_estimator

__all__ = [
    "acf",
    "hill_estimator",
    "ljung_box",
    "log_returns",
    "moments",
    "trade_sign_autocorr",
    "volatility_clustering_decay",
    "zumbach_asymmetry",
]
