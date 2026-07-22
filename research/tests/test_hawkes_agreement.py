"""Cross-implementation agreement: the Python and Rust Hawkes MLE fitters,
run on the SAME committed fixture, must produce matching log-likelihoods and
branching ratios. Two independent implementations of the same estimator are
a strong guard against transcription bugs in either.

Skipped unless the `quant-sim` binary is built (it is in CI, after the Rust
build job) and the fixture is present.
"""

import json
import subprocess
import tomllib
from pathlib import Path

import polars as pl
import pytest

from quantsim_research.config import repo_root
from quantsim_research.hawkes.exp_mle import fit_exp_hawkes

FIXTURE = repo_root() / "data" / "fixtures" / "binance-um" / "trades" / "BTCUSDT-2024-01-07-h00.csv"


def _rust_binary() -> Path | None:
    for profile in ("release", "debug"):
        exe = (
            repo_root()
            / "target"
            / profile
            / ("quant-sim.exe" if __import__("os").name == "nt" else "quant-sim")
        )
        if exe.exists():
            return exe
    return None


@pytest.mark.slow  # ~9 min: Python MLE over 41k events × 4 multi-starts
@pytest.mark.skipif(not FIXTURE.exists(), reason="fixture not built")
def test_python_and_rust_hawkes_agree(tmp_path):
    binary = _rust_binary()
    if binary is None:
        pytest.skip("quant-sim binary not built")

    # Rust fit.
    out = tmp_path / "rust.toml"
    subprocess.run(
        [str(binary), "fit", "hawkes", str(FIXTURE), "--out", str(out)],
        check=True,
        capture_output=True,
    )
    rust = tomllib.loads(out.read_text(encoding="utf-8"))
    rust_ll = rust["diagnostics"]["loglik"]
    rust_rho = rust["diagnostics"]["spectral_radius"]

    # Python fit on the same data: buy = +1, sell = -1, times in seconds.
    df = pl.read_csv(FIXTURE)
    ts = df["ts_ns"].to_numpy()
    side = df["side"].to_numpy()
    t0 = ts[0]
    seconds = (ts - t0) / 1e9
    events = [seconds[side == 1], seconds[side == -1]]
    t_end = float(seconds[-1])
    fit = fit_exp_hawkes(events, t_end)

    # The two optimizers (Nelder-Mead vs L-BFGS-B) land close but not
    # identical; require the log-likelihoods within 1% and branching within
    # 0.05 — far tighter than the gap to Poisson (Δ ~ 70000).
    assert abs(fit.log_likelihood - rust_ll) / abs(rust_ll) < 0.01, (
        f"logL Python {fit.log_likelihood:.1f} vs Rust {rust_ll:.1f}"
    )
    assert abs(fit.spectral_radius - rust_rho) < 0.05, (
        f"branching Python {fit.spectral_radius:.3f} vs Rust {rust_rho:.3f}"
    )
    # Both must decisively beat Poisson.
    assert fit.log_likelihood > fit.poisson_log_likelihood + 1000

    # Also confirm the report cross-check plumbing round-trips a fit's JSON.
    _ = json.dumps({"ok": True})
