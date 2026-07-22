"""Cross-check a Rust run-report JSON against the independent Python metrics."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from quantsim_research.metrics.sharpe import (
    probabilistic_sharpe,
    sharpe_ratio,
    sortino_ratio,
)

# Tolerances from docs/interchange.md.
REL_TOL_SHARPE = 1e-9
ABS_TOL_PSR = 1e-7


@dataclass
class CheckResult:
    strategy: str
    metric: str
    rust: float
    python: float
    ok: bool


def crosscheck_report(path: Path) -> list[CheckResult]:
    """Recompute Sharpe/Sortino/PSR from the report's `returns[]` and compare
    to the Rust-reported values."""
    report = json.loads(Path(path).read_text(encoding="utf-8"))
    results: list[CheckResult] = []
    for block in report["strategies"]:
        results.extend(_check_block(block))
    return results


def _check_block(block: dict) -> list[CheckResult]:
    returns = np.array(block["returns"], dtype=float)
    metrics = block["metrics"]
    strategy = block["name"]
    out: list[CheckResult] = []

    def check(name: str, rust_val, python_val, tol, *, relative: bool) -> None:
        if rust_val is None or python_val is None:
            return
        diff = abs(rust_val - python_val)
        denom = max(abs(rust_val), 1e-30) if relative else 1.0
        ok = diff / denom <= tol if relative else diff <= tol
        out.append(CheckResult(strategy, name, rust_val, python_val, ok))

    check("sharpe", metrics.get("sharpe"), sharpe_ratio(returns), REL_TOL_SHARPE, relative=True)
    check("sortino", metrics.get("sortino"), sortino_ratio(returns), REL_TOL_SHARPE, relative=True)
    psr = probabilistic_sharpe(returns, 0.0)
    check("psr", metrics.get("psr"), psr, ABS_TOL_PSR, relative=False)
    return out


def all_ok(results: list[CheckResult]) -> bool:
    return all(r.ok for r in results)
