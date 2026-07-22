"""Generate the committed research notebooks (01-04) from source.

Run: uv run python notebooks/build_notebooks.py
Each notebook is papermill-parameterized (FIXTURE_ONLY) and runs offline on
the committed fixture in well under CI budget. Regenerating and committing
keeps notebooks reviewable as code.
"""

from pathlib import Path

import nbformat as nbf

HERE = Path(__file__).parent

PREAMBLE = """\
import numpy as np
import polars as pl
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

from quantsim_research.config import repo_root

FIXTURE = repo_root() / "data" / "fixtures" / "binance-um" / "trades" / "BTCUSDT-2024-01-07-h00.csv"
ARTIFACTS = repo_root() / "artifacts" / "notebooks"
ARTIFACTS.mkdir(parents=True, exist_ok=True)
df = pl.read_csv(FIXTURE)
ts_ns = df["ts_ns"].to_numpy()
price = df["price_e8"].to_numpy() / 1e8
side = df["side"].to_numpy()
print(f"fixture: {len(df)} trades, {(ts_ns[-1]-ts_ns[0])/1e9:.0f}s span")
"""


KERNELSPEC = {
    "kernelspec": {"name": "python3", "display_name": "Python 3", "language": "python"},
    "language_info": {"name": "python", "version": "3.13"},
}


def nb(title: str, cells: list[tuple[str, str]]) -> nbf.NotebookNode:
    notebook = nbf.v4.new_notebook()
    notebook.metadata.update(KERNELSPEC)
    notebook.cells.append(nbf.v4.new_markdown_cell(title))
    params = nbf.v4.new_code_cell("FIXTURE_ONLY = True  # papermill parameter")
    params.metadata["tags"] = ["parameters"]
    notebook.cells.append(params)
    for kind, src in cells:
        if kind == "md":
            notebook.cells.append(nbf.v4.new_markdown_cell(src))
        else:
            notebook.cells.append(nbf.v4.new_code_cell(src))
    return notebook


NB01 = nb(
    "# 01 — Stylized facts of BTCUSDT perp trades (fixture scale)\n\n"
    "The Cont (2001) battery on the committed 1-hour fixture. At fixture "
    "scale the estimates are noisy — the full study window (Jan 2024) "
    "sharpens them; the *code path* is identical.",
    [
        ("code", PREAMBLE),
        (
            "md",
            "## Return moments and fat tails\n"
            "1-second log-returns: excess kurtosis, Jarque-Bera, Hill tail index.",
        ),
        (
            "code",
            """\
from quantsim_research.stylized_facts.returns import excess_kurtosis_by_scale, moments
from quantsim_research.stylized_facts.tails import hill_estimator

# Resample to a 1-second last-price grid.
sec = ((ts_ns - ts_ns[0]) / 1e9).astype(int)
last_in_sec = {}
for s, p in zip(sec, price, strict=True):
    last_in_sec[s] = p
grid = np.array([last_in_sec[k] for k in sorted(last_in_sec)])
r1 = np.diff(np.log(grid))
m = moments(r1)
jb, jb_p = m.jarque_bera()
hill = hill_estimator(r1, tail_fraction=0.05)
print(f"n={m.n}  std={m.std:.2e}  skew={m.skewness:.2f}  ex.kurt={m.excess_kurtosis:.2f}")
print(f"Jarque-Bera={jb:.0f} (p={jb_p:.1e})  Hill alpha={hill.alpha:.2f} (k={hill.k})")
agg = excess_kurtosis_by_scale(grid, [1, 5, 15, 60])
print("aggregational gaussianity (ex.kurt by scale):", {k: round(v, 2) for k, v in agg.items()})
""",
        ),
        (
            "md",
            "## Volatility clustering vs no linear autocorrelation",
        ),
        (
            "code",
            """\
from quantsim_research.stylized_facts.autocorr import acf, bartlett_band, ljung_box

lags = 60
rho_r = acf(r1, lags)
rho_abs = acf(np.abs(r1), lags)
band = bartlett_band(r1.size)
fig, ax = plt.subplots(1, 2, figsize=(11, 3.5))
ax[0].bar(range(1, lags + 1), rho_r)
ax[0].axhline(band, ls='--', c='r')
ax[0].axhline(-band, ls='--', c='r')
ax[0].set_title("ACF raw returns (should die fast)")
ax[1].bar(range(1, lags + 1), rho_abs); ax[1].axhline(band, ls='--', c='r')
ax[1].set_title("ACF |returns| (clustering)")
fig.tight_layout(); fig.savefig(ARTIFACTS / "01_acf.png", dpi=120)
print("Ljung-Box raw:", ljung_box(r1, 20))
print("Ljung-Box |r|:", ljung_box(np.abs(r1), 20))
""",
        ),
        (
            "md",
            "## Trade-sign autocorrelation and the Zumbach asymmetry\n"
            "Sign clustering is strong and slow (order splitting). Zumbach "
            "asymmetry is the statistic our linear-Hawkes simulator is "
            "*preregistered to miss* — quadratic Hawkes (Stage 2) targets it.",
        ),
        (
            "code",
            """\
from quantsim_research.stylized_facts.orderflow import trade_sign_autocorr, zumbach_asymmetry

sign_acf = trade_sign_autocorr(side.astype(float), max_lag=200)
print(f"sign ACF lag1={sign_acf[0]:.3f} lag10={sign_acf[9]:.3f} lag100={sign_acf[99]:.3f}")
z = zumbach_asymmetry(grid, window=30)
print(f"Zumbach: trend->vol={z.trend_predicts_vol:.3f}  vol->trend={z.vol_predicts_trend:.3f}  "
      f"asymmetry={z.asymmetry:+.3f}")
""",
        ),
    ],
)

NB02 = nb(
    "# 02 — Hawkes fit and goodness-of-fit on real trade arrivals\n\n"
    "Fit the 2-dim (buy/sell aggressor) exponential Hawkes to the fixture "
    "hour and test it with Ogata's time-rescaling residuals. The Rust CLI "
    "(`quant-sim fit hawkes`) fits the same model; a slow test holds the two "
    "implementations to matching likelihoods.",
    [
        ("code", PREAMBLE),
        (
            "code",
            """\
from quantsim_research.hawkes.exp_mle import fit_exp_hawkes

seconds = (ts_ns - ts_ns[0]) / 1e9
events = [seconds[side == 1], seconds[side == -1]]
t_end = float(seconds[-1])
if FIXTURE_ONLY:
    # Subsample to the first 10 minutes so the notebook executes in seconds.
    events = [e[e < 600.0] for e in events]
    t_end = 600.0
fit = fit_exp_hawkes(events, t_end)
print(f"mu={fit.mu.round(3)}  beta={fit.beta:.1f}/s  rho(G)={fit.spectral_radius:.3f}")
print(f"logL={fit.log_likelihood:.1f} vs Poisson {fit.poisson_log_likelihood:.1f} "
      f"(delta={fit.log_likelihood - fit.poisson_log_likelihood:.0f})")
""",
        ),
        (
            "md",
            "## Time-rescaling residuals\n"
            "Under the fitted model, compensator-transformed inter-arrivals "
            "should be iid Exp(1): KS test + QQ plot.",
        ),
        (
            "code",
            """\
from scipy import stats

from quantsim_research.hawkes.diagnostics import time_rescaling_residuals

res = time_rescaling_residuals(events, fit.mu, fit.alpha, fit.beta, dim=0)
print(f"KS={res.ks_statistic:.4f} p={res.ks_pvalue:.3f}  mean tau={res.residuals.mean():.3f}")
theo = stats.expon.ppf((np.arange(1, res.residuals.size + 1) - 0.5) / res.residuals.size)
fig, ax = plt.subplots(figsize=(4.5, 4.5))
ax.plot(theo, np.sort(res.residuals), '.', ms=2)
lim = max(theo.max(), res.residuals.max())
ax.plot([0, lim], [0, lim], 'r--', lw=1)
ax.set_xlabel("Exp(1) quantiles"); ax.set_ylabel("rescaled residuals"); ax.set_title("QQ")
fig.tight_layout(); fig.savefig(ARTIFACTS / "02_qq.png", dpi=120)
""",
        ),
    ],
)

NB03 = nb(
    "# 03 — Simulator validation: real vs simulated stylized facts\n\n"
    "The headline artifact: the same battery on (a) the real fixture and "
    "(b) trades exported from the Rust Hawkes simulator (`quant-sim run "
    "scenarios/hawkes_demo.toml` + `export-events`). Preregistered expected "
    "failures of the v1 exp-kernel simulator, stated before results: "
    "long-memory of signs, volatility roughness, Zumbach asymmetry.",
    [
        ("code", PREAMBLE),
        (
            "code",
            """\
from quantsim_research.stylized_facts.autocorr import acf, ljung_box
from quantsim_research.stylized_facts.orderflow import trade_sign_autocorr, zumbach_asymmetry
from quantsim_research.stylized_facts.returns import moments


def battery(ts, px, sd, label):
    sec = ((ts - ts[0]) / 1e9).astype(int)
    last = {}
    for s, p in zip(sec, px, strict=True):
        last[s] = p
    grid = np.array([last[k] for k in sorted(last)])
    r = np.diff(np.log(grid))
    m = moments(r)
    rho_abs = acf(np.abs(r), 20)
    sacf = trade_sign_autocorr(sd.astype(float), max_lag=50)
    z = zumbach_asymmetry(grid, window=30)
    return {
        "label": label,
        "ex_kurt": round(m.excess_kurtosis, 2),
        "ljung_absr_p": f"{ljung_box(np.abs(r), 10).p_value:.1e}",
        "acf_abs_lag1": round(float(rho_abs[0]), 3),
        "sign_acf_lag1": round(float(sacf[0]), 3),
        "sign_acf_lag20": round(float(sacf[19]), 3),
        "zumbach": round(z.asymmetry, 3) if np.isfinite(z.asymmetry) else None,
    }


rows = [battery(ts_ns, price, side, "real BTCUSDT (1h)")]
""",
        ),
        (
            "code",
            """\
# Simulated trades, if a Rust export is present (guarded so the notebook
# always runs offline; CI generates the export before executing).
sim_csv = repo_root() / "runs" / "hawkes_demo" / "trades.csv"
if sim_csv.exists():
    sim = pl.read_csv(sim_csv)
    rows.append(
        battery(
            sim["ts_ns"].to_numpy(),
            sim["price_e8"].to_numpy() / 1e8,
            sim["side"].to_numpy(),
            "sim exp-Hawkes",
        )
    )
else:
    print("no sim export found - run `quant-sim run scenarios/hawkes_demo.toml` "
          "and `quant-sim export-events` first for the full comparison")
table = pl.DataFrame(rows)
print(table)
table.write_csv(ARTIFACTS / "03_real_vs_sim.csv")
""",
        ),
        (
            "md",
            "### Reading the table\n"
            "- **Volatility clustering / fat tails**: exp-Hawkes partially "
            "reproduces (clustered arrivals ⇒ clustered activity).\n"
            "- **Sign long-memory**: expected shortfall — sim sign ACF decays "
            "exponentially, real decays as a power law (order splitting).\n"
            "- **Zumbach**: expected ≈ 0 in sim (linear Hawkes is TR-symmetric); "
            "real is positive. Stage 2's quadratic Hawkes targets exactly this.",
        ),
    ],
)

NB04 = nb(
    "# 04 — Is volatility rough? Hurst estimates, real vs sim\n\n"
    "H of the realized-vol proxy via DFA, R/S and the GJR smoothness "
    "regression. Preregistered: real BTC vol is rough (H well below 0.5); "
    "the v1 exp-kernel sim is NOT rough (H ≈ 0.5) — the Jusselin–Rosenbaum "
    "prediction our Stage-2 power-law-kernel experiment then attacks.",
    [
        ("code", PREAMBLE),
        (
            "code",
            """\
from quantsim_research.hurst.estimators import dfa_hurst, gjr_smoothness_hurst, rescaled_range_hurst
from quantsim_research.hurst.vol_proxy import realized_vol

# Fixture-scale: 10s RV bins over the hour (coarse; the full study uses
# 5-min bins over a month). Small-sample caveats apply to every number here.
rv = realized_vol(ts_ns, price, bin_secs=10.0)
log_rv = np.log(rv[rv > 0])
print(f"{rv.size} RV bins")
print(f"DFA H       = {dfa_hurst(log_rv):.3f}")
print(f"R/S H       = {rescaled_range_hurst(log_rv):.3f}")
print(f"GJR H       = {gjr_smoothness_hurst(log_rv, max_lag=min(50, rv.size // 5)):.3f}")
""",
        ),
        (
            "code",
            """\
# Calibration sanity inline: the estimators recover known H on synthetic fGn.
rng = np.random.default_rng(0)


def fgn(n, h):
    def g(k):
        return 0.5 * (abs(k - 1) ** (2 * h) - 2 * abs(k) ** (2 * h) + abs(k + 1) ** (2 * h))
    gg = np.array([g(k) for k in range(n)])
    r = np.concatenate([gg, gg[-2:0:-1]])
    lam = np.fft.fft(r).real
    lam[lam < 0] = 0
    m_ = r.size
    w = rng.standard_normal(m_) + 1j * rng.standard_normal(m_)
    return np.fft.fft(np.sqrt(lam / (2 * m_)) * w).real[:n]


for h_true in (0.1, 0.5):
    print(f"H={h_true}: DFA -> {dfa_hurst(fgn(4096, h_true)):.3f}")
""",
        ),
    ],
)


def main() -> None:
    for name, notebook in [
        ("01-stylized-facts.ipynb", NB01),
        ("02-hawkes-fit.ipynb", NB02),
        ("03-simulator-validation.ipynb", NB03),
        ("04-hurst.ipynb", NB04),
    ]:
        nbf.write(notebook, HERE / name)
        print(f"wrote {name}")


if __name__ == "__main__":
    main()
