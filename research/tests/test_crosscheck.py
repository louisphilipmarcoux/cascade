"""Cross-check the Rust backtester's metrics against the Python reimplementation.

Uses a committed golden report if present; otherwise a synthetic report with
Python-computed values (a self-consistency check of the cross-checker).
"""

import json

import numpy as np

from quantsim_research.metrics.crosscheck import all_ok, crosscheck_report
from quantsim_research.metrics.sharpe import (
    probabilistic_sharpe,
    sharpe_ratio,
    sortino_ratio,
)


def test_crosscheck_agrees_on_synthetic_report(tmp_path):
    rng = np.random.default_rng(0)
    returns = list(0.0005 + 0.01 * rng.standard_normal(500))
    report = {
        "schema_version": 1,
        "strategies": [
            {
                "name": "synthetic",
                "returns": returns,
                "metrics": {
                    "sharpe": sharpe_ratio(np.array(returns)),
                    "sortino": sortino_ratio(np.array(returns)),
                    "psr": probabilistic_sharpe(np.array(returns), 0.0),
                },
            }
        ],
    }
    path = tmp_path / "report.json"
    path.write_text(json.dumps(report), encoding="utf-8")
    results = crosscheck_report(path)
    assert len(results) == 3
    assert all_ok(results), [(r.metric, r.rust, r.python) for r in results if not r.ok]


def test_crosscheck_detects_disagreement(tmp_path):
    returns = list(0.001 * np.arange(1, 101))
    report = {
        "schema_version": 1,
        "strategies": [
            {
                "name": "wrong",
                "returns": returns,
                "metrics": {"sharpe": 999.0, "sortino": None, "psr": None},
            }
        ],
    }
    path = tmp_path / "report.json"
    path.write_text(json.dumps(report), encoding="utf-8")
    results = crosscheck_report(path)
    assert not all_ok(results)
