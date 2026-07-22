"""Independent reimplementation of the Rust backtester's honest statistics.

This is a *cross-check*: the Sharpe / probabilistic-Sharpe / deflated-Sharpe
in `crates/backtester/src/metrics` are re-derived here from the authoritative
per-period returns in the run-report JSON, and the two must agree within
tolerance (docs/interchange.md). Two independent implementations of the same
formula catch transcription bugs neither would alone.
"""

from quantsim_research.metrics.sharpe import (
    deflated_sharpe,
    probabilistic_sharpe,
    sharpe_ratio,
    sortino_ratio,
)

__all__ = [
    "deflated_sharpe",
    "probabilistic_sharpe",
    "sharpe_ratio",
    "sortino_ratio",
]
