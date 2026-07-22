# quant-sim

An integrated market simulation, matching engine & research platform: a Rust
workspace combining a formally-verified limit order book engine, a
deterministic Hawkes-driven market simulator, and a statistically honest
backtesting framework — with a Python research layer that validates the
simulator against real market data.

Every performance or correctness statement below is backed by a benchmark, a
test, a proof harness, or a citation. Nothing here is aspirational: the
numbers are measured, the failures are reported, and the claims the project
*cannot* yet make are listed as explicitly as the ones it can.

## What it does, in one run

```sh
quant-sim run scenarios/as_vs_naive.toml --seed 42
```

simulates 600 seconds of a self-exciting (Hawkes) crypto-like market —
8,013,463 engine events — through a price-time-priority matching engine with
self-match prevention, latency-modeled order paths and adverse fee rounding,
runs two market-making strategies against identical flow, and prints
per-strategy PnL with probabilistic Sharpe ratios. Byte-identical on every
rerun: the event stream's BLAKE3 hash (`b8966b4d…`) is printed and
`--verify-hash` re-runs and compares.

## Verification ladder

The core claim is not "it's fast" but "it's *right*, provably, at several
levels — and the proofs transfer":

| Level | Tool | What is established |
| --- | --- | --- |
| Bounded proof | [Kani](https://model-checking.github.io/kani/) (CBMC) | Volume conservation (K1), no crossed book (K2), no over-fill (K3), FOK atomicity (K4), fee-rounding bounds (K5), checked-money algebra (K6) — for **all** op sequences within bounds on `ArrayBook` |
| Differential | proptest, 256 seq/run | `ArrayBook` ≡ `BTreeBook` ≡ `LadderBook`: identical event streams and snapshots on random op sequences — Kani's confidence transfers to both production books |
| Property | proptest (P1–P23) | Book structure, conservation, priority, STP, accounting identity `equity = realized + unrealized − fees` (exact in i128, no epsilon), serde round-trips, determinism |
| Statistical | fixed-seed golden hashes | Whole-simulation event streams pinned (`32aa58c4…` for the DES suite); replay reproduces a real 41,473-trade tape exactly |
| Cross-impl | Rust ↔ Python | Hawkes MLE: two independent implementations agree on real data (logL within 1%, branching within 0.05); PSR/Sharpe within 1e-9 |

Determinism is enforced structurally, not hoped for: virtual clock only
(`Instant::now` is a clippy error in sim paths), no `HashMap` in observable
paths, ChaCha12 per-component substreams, FIFO tie-breaks in the event queue,
`libm` for portable transcendentals.

## Measured performance

Single-threaded, full engine (validation + matching + event emission),
1,000,000 requests after 200k warmup. AMD Ryzen 5 5500U (laptop), Windows 11,
Rust 1.96, release + debuginfo. Reproduce: `cargo bench -p matching-engine`.

| Workload | Book | Throughput | p50 | p99 | p99.9 |
| --- | --- | --- | --- | --- | --- |
| uniform (wide book) | BTree | 2.63M req/s | 300 ns | 900 ns | 1.7 µs |
| uniform | Ladder | 2.84M req/s | 300 ns | 700 ns | 1.3 µs |
| near-touch (realistic) | BTree | 4.37M req/s | 200 ns | 600 ns | 900 ns |
| near-touch | Ladder | **4.88M req/s** | **100 ns** | **500 ns** | 800 ns |
| sweepy (deep takes) | BTree | 4.02M req/s | 200 ns | 600 ns | 1.2 µs |
| sweepy | Ladder | 3.92M req/s | 200 ns | 700 ns | 1.3 µs |

The contiguous price-band `LadderBook` beats the `BTreeMap` ladder where
books are dense near the touch (the case realistic flow produces) and ties or
slightly loses on sparse sweep-heavy flow — the trade-off is measured, not
asserted. All three books emit identical event streams on every benchmarked
workload (asserted by count above, by content in the differential suite).

## Research results (real data, committed fixture)

Everything below reproduces offline from the committed fixture
(BTCUSDT USD-M perp, 2024-01-07 00:00–01:00 UTC, 41,473 trades, SHA-256
pinned) via `uv run pytest` and notebooks 01–04.

**Stylized facts** (notebook 01): excess kurtosis **43.6** at 1 s falling to
0.77 at 60 s (aggregational gaussianity); Hill tail index **α = 2.79** (the
"cubic law"); |returns| Ljung-Box p ≈ 1e-149 vs raw-returns p ≈ 1e-10
(volatility clusters, returns barely autocorrelate); trade-sign ACF 0.38 at
lag 1 decaying slowly to 0.09 at lag 100 (order splitting); Zumbach
time-reversal asymmetry **+0.048** (trends predict volatility more than the
reverse).

**Hawkes** (notebook 02): 2-dim exponential-kernel MLE on the hour gives
branching ratio ρ(G) ≈ 0.42–0.50, β ≈ 120–250/s, and beats a Poisson model
by Δ logL ≈ 70,000 over the full fixture. Ogata's time-rescaling test then
**rejects** the exponential kernel on real data — the correct verdict, since
single-timescale kernels can't capture power-law memory. On synthetic
exp-Hawkes data the same test passes and the fitter recovers true parameters.

**Volatility is rough** (notebook 04): the Gatheral–Jaisson–Rosenbaum
smoothness regression on realized volatility gives **H ≈ 0.08** on the real
fixture — rough, exactly as the literature finds — while DFA on vol *levels*
gives 0.82 (persistence; both facts are real and distinct). The estimators
recover known H on exact synthetic fBm to ±0.12. Preregistered: our v1
exponential-kernel simulator will *not* reproduce H < 0.5; the Stage-2
near-critical power-law-kernel experiment tests whether the
Jusselin–Rosenbaum mechanism closes that gap.

**Strategies, honestly** (paired seeds, identical flow and frictions):

| Comparison | Result |
| --- | --- |
| Avellaneda-Stoikov vs naive MM | Under adversarial Hawkes flow **both lose**; A-S loses ~15× less (−1,313 vs −20,678 equity units) with a third of the drawdown. The paper's edge is inventory control, and that is what shows up. |
| Almgren-Chriss vs TWAP | A-C realizes −5.9 vs TWAP's −56.8 implementation shortfall on the demo config (13 vs 102 child fills). |
| OU pairs on ground-truth cointegration | Extracts the planted mean-reversion edge (+279k, PSR 1.00) from a synthetic cointegrated pair — a known-answer test of the whole stack. |
| Deflated Sharpe discipline | The spread study runs K=6 real trials; DSR(best) = 0.72 is computed **from those trials**. A single run reports DSR = null ("K=1") — the tool refuses to pretend one backtest is evidence. |

## Layout

- [crates/sim-core](crates/sim-core) — fixed-point money types (i64 ticks / i128 cash), virtual time, event vocabulary, seeded RNG streams
- [crates/matching-engine](crates/matching-engine) — the engine + three `Book` implementations and the proof/differential harness
- [crates/market-sim](crates/market-sim) — discrete-event simulator: Hawkes/Poisson/replay/cointegrated-pair flow, latency & fault injection, Hawkes MLE
- [crates/backtester](crates/backtester) — event-driven strategies over delayed views, exact accounting, PSR/DSR metrics
- [crates/strategies](crates/strategies) — Avellaneda-Stoikov, OU pairs, Almgren-Chriss/TWAP, estimators
- [crates/cli](crates/cli) — the `quant-sim` binary: scenario runner/studies, `fit hawkes|ou`, exports, hash verification
- [research/](research) — uv-managed Python: pinned data pipeline, stylized facts, Hawkes diagnostics, Hurst, metrics cross-check, notebooks
- [docs/](docs) — [spec](docs/spec.md), [architecture](docs/ARCHITECTURE.md), [determinism](docs/determinism.md), [interchange contracts](docs/interchange.md), [reading log](docs/papers.md)

## Quickstart

```sh
just setup   # one-time: nextest, deny, mutants
just ci      # fmt + clippy -D warnings + tests + docs — identical to CI
quant-sim run scenarios/ou_pairs_demo.toml       # a full backtest in seconds
cd research && uv sync && uv run pytest          # the Python battery
```

Linux and Windows are both first-class: the full suite runs on both in CI, a
`Dockerfile` reproduces the pipeline on linux/amd64, and Kani proofs run in
CI (ubuntu) and under WSL locally.

## Known limitations (stated before you find them)

- **Simulated depth is synthetic.** Free tick-level L2 does not exist for
  crypto; real L1 (bookTicker) and band depth calibrate the placement model,
  and full L2 is generated by our own flow models. Only L1/imbalance
  statistics are validated against real quotes. A live recorder has been
  capturing true depth-deltas since M7a for the Stage-3 exact-replay mode.
- **The exp-kernel Hawkes simulator misses preregistered statistics**: sign
  long-memory, volatility roughness, Zumbach asymmetry. These are the
  Stage-2 targets (power-law kernels, quadratic Hawkes), not surprises.
- **Historical-replay fills are estimates.** The replay source frames real
  trades with synthetic quotes; queue-position uncertainty is the Stage-3
  agenda (model family + sensitivity band, not one arbitrary choice).
- **Byte-identical determinism is claimed per binary/platform.** Cross-OS
  hash equality is tested in CI and reported, not promised: strategy-side
  `f64` may legally differ across targets.
