"""Hawkes fitting and goodness-of-fit, independent of the Rust fitter.

The Python side fits the same 2-dim exponential-kernel MLE (scipy) and adds
diagnostics the Rust CLI doesn't: the time-rescaling goodness-of-fit test
(transformed inter-arrivals should be iid Exp(1); KS + QQ). A
cross-implementation agreement test holds the Rust and Python fits to the
same log-likelihood on the same data.
"""

from quantsim_research.hawkes.diagnostics import time_rescaling_residuals
from quantsim_research.hawkes.exp_mle import fit_exp_hawkes

__all__ = ["fit_exp_hawkes", "time_rescaling_residuals"]
