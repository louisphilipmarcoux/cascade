# Rust ↔ Python interchange contracts

Everything that crosses the Rust/Python boundary is one of the versioned
formats below. Schema changes bump `schema_version` and keep a reader for the
previous version for one milestone.

## Canonical cross-boundary data format: scaled-i64 CSV

Deterministic, float-free, dependency-light (keeps arrow out of the default
Rust build). RFC 4180, header row, LF line endings, UTF-8, UTC epoch
**nanoseconds**. All prices/quantities are integers scaled by 1e8 (`_e8`).
Python-internal analysis uses hive-partitioned Parquet (Float64, polars); the
fixture builder emits both twins. Rust Parquet support exists behind the
off-by-default `parquet` feature.

### `trades` (Python → Rust replay input; Rust market-sim → Python sim output — identical schema)

| column         | type | meaning                                       |
| -------------- | ---- | --------------------------------------------- |
| `ts_ns`        | i64  | event time, epoch ns UTC (sim: virtual ns)    |
| `side`         | i8   | aggressor: +1 buy, −1 sell                    |
| `price_e8`     | i64  | trade price × 1e8                             |
| `qty_e8`       | i64  | base-asset quantity × 1e8                     |
| `agg_trade_id` | i64  | source aggregate-trade id (sim: monotone id)  |

Source note: one Binance aggTrade = the fills of one taker order at one price
⇒ best free proxy for a market-order arrival. `is_buyer_maker = true` ⇒ the
aggressor sold ⇒ `side = -1`.

### `quotes_l1` (Python → Rust; from Binance USD-M `bookTicker`)

| column       | type | meaning                    |
| ------------ | ---- | -------------------------- |
| `ts_ns`      | i64  | transaction time, epoch ns |
| `update_id`  | i64  | venue update id            |
| `bid_e8`     | i64  | best bid price × 1e8       |
| `bid_qty_e8` | i64  | best bid qty × 1e8         |
| `ask_e8`     | i64  | best ask price × 1e8       |
| `ask_qty_e8` | i64  | best ask qty × 1e8         |

### Rust run exports (→ Python)

- `events.csv` / `events.jsonl` — flattened engine event stream (nullable
  per-variant columns; `seq`, `ts_ns`, `symbol`, `type`, then variant fields).
- `fills.csv` — `ts_ns, trade_id, symbol, owner, side, price_e8, qty_e8, fee_e8, liquidity` (`maker`|`taker`).
- `equity.csv` — `ts_ns, equity_e8, cash_e8, inventory_lots, mark_e8`.
- `l1.csv` — `ts_ns, bid_e8, bid_qty_e8, ask_e8, ask_qty_e8` (projection output).
- `trades.csv` — the `trades` schema above (so real-vs-sim analysis is symmetric).

## Instruments (`data/instruments.toml`, committed)

```toml
schema_version = 1

[[instrument]]
symbol      = "BTCUSDT"
exchange    = "binance-um"
tick_size   = "0.10"    # quote per tick (decimal string, parsed exactly)
lot_size    = "0.001"   # base per lot
```

Contract test on the Rust side: for the whole fixture,
`round(price / tick_size)` round-trips exactly (no lossy prices), and
`tick_size_e8 × lot_size_e8 ≡ 0 (mod 1e8)` so all notional arithmetic is exact
integer math.

## Hawkes params artifact (TOML, written by BOTH fitters)

Both the Rust CLI (`quant-sim fit hawkes`) and the Python research package fit
the exponential-kernel MLE; a cross-implementation agreement test holds them to
the same log-likelihood/params within tolerance. Rust's simulator reads
`[baseline]`, `[kernel]`, `[marks]` only; `[diagnostics]`/`[provenance]` are
for reports and the paper.

```toml
schema_version = 1
model = "hawkes_exp_mv"          # Stage 2 adds "hawkes_sumexp_mv" (arrays gain an M axis)
dims = ["buy", "sell"]
symbol = "BTCUSDT"
exchange = "binance-um"
time_unit = "seconds"

[fitted_on]
start = "2024-06-01T00:00:00Z"
end = "2024-06-30T23:59:59Z"
n_events = 0
window = "4h"

[baseline]
mu = [0.0, 0.0]                  # events/sec per dim

[kernel]                         # alpha[m][n]: excitation of dim m by dim n (1/sec); beta: decay (1/sec)
alpha = [[0.0, 0.0], [0.0, 0.0]]
beta  = [[0.0, 0.0], [0.0, 0.0]]

[marks]                          # trade-size distribution per dim
size_dist = "lognormal"
params = [[0.0, 0.0], [0.0, 0.0]]  # (mu, sigma) of log size

[diagnostics]                    # ignored by the Rust simulator
loglik = 0.0
aic = 0.0
branching_matrix = [[0.0, 0.0], [0.0, 0.0]]
spectral_radius = 0.0
ks_pvalue = [0.0, 0.0]
stability_ok = true
# Stage 2 adds [diagnostics.posterior] with credible intervals.

[provenance]
git_commit = ""
data_manifest_entries = [""]
created_utc = ""
```

Writers MUST refuse to emit a file with spectral radius ≥ 1 (unstable fit).

## Backtest report (JSON, Rust → Python cross-check)

`RunReport` (`schema_version: 1`): run id, seed, `scenario_hash` (BLAKE3 of the
canonicalized scenario), `events_hash` (BLAKE3 of the event stream),
git commit, sim span, config echo, and per-strategy blocks with
`pnl {realized_e8, unrealized_e8, fees_e8}`, `period_ns`, the authoritative
per-period `returns[]`, `metrics {sharpe, sortino, psr, dsr?, max_drawdown, hit_rate, turnover, …}`,
and artifact paths. `StudyReport` adds `trials {n_trials, trial_sharpes[]}` and
best-variant DSR computed from those real trials (single runs report
`dsr: null` with reason `"K=1"`).

Python (`quantsim_research.metrics.crosscheck`) recomputes Sharpe/Sortino/max-DD
from `returns[]` and asserts agreement ≤ 1e-9 relative (PSR/DSR ≤ 1e-7,
tolerance for Φ implementation differences).

## Data manifest (`data/manifest.toml`, committed)

```toml
[[archive]]
url = "https://data.binance.vision/data/futures/um/daily/aggTrades/BTCUSDT/BTCUSDT-aggTrades-2024-06-02.zip"
sha256 = "…64 hex…"
size_bytes = 0
binance_checksum_verified = true
downloaded_at = "2026-07-18T00:00:00Z"
```

Re-downloading a pinned URL that hashes differently is a **hard error**
(upstream-mutation tripwire). `quantsim-data verify` re-hashes everything and
exits nonzero on any mismatch (CI-usable).
