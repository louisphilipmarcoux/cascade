# quant-sim

An integrated market simulation, matching engine & research platform: a Rust
workspace combining a formally-harnessed limit order book engine, a
deterministic Hawkes-driven market simulator, and a statistically honest
backtesting and research framework, with a Python research layer.

> Status: under active construction, milestone by milestone. Every performance
> or correctness statement in this repository is backed by a benchmark, a test,
> a proof harness, or a citation — see [docs/spec.md](docs/spec.md) for the
> project philosophy and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the
> system design.

## Layout

- `crates/sim-core` — deterministic substrate: fixed-point market types, virtual time, event vocabulary, seeded RNG streams
- `crates/matching-engine` — price-time-priority limit order book (property-tested + Kani-proved invariants)
- `crates/market-sim` — discrete-event simulator: Hawkes/Poisson/replay order flow, latency & fault injection
- `crates/backtester` — event-driven strategy execution with honest statistics (deflated Sharpe over real trials)
- `crates/strategies` — Avellaneda-Stoikov, OU pairs, Almgren-Chriss, baselines
- `crates/cli` — the `quant-sim` binary: scenario runner, fitting, exports
- `research/` — uv-managed Python package: data pipeline, stylized-facts validation, Hawkes diagnostics, notebooks

## Quickstart

```sh
just setup   # one-time: nextest, deny, mutants
just ci      # fmt + clippy + tests + docs, identical to CI
```

Linux and Windows are both first-class: the full test suite runs on both in CI,
and a `Dockerfile` reproduces the entire pipeline on linux/amd64.
