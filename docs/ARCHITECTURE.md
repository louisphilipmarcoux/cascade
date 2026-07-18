# Architecture

quant-sim is an event-sourced, deterministic discrete-event market simulator
with a formally-harnessed matching engine at its core. This note records the
load-bearing design decisions; per-crate READMEs record local tradeoffs.

## Event sourcing

The matching engine's **only output is an append-only, totally ordered event
stream** (`sim_core::events`). Everything downstream is a consumer/projection:

- backtester accounting (positions, PnL, fees) folds over fills,
- L2 book views are *projections* rebuilt from order lifecycle events
  (losslessness is property-tested: projection == engine book at every step),
- research exports (CSV/JSONL) serialize the same stream,
- later: the WASM live demo renders it, and the distributed variant replicates it.

The engine crates (`sim-core`, `matching-engine`) have **no filesystem, clock,
threading, or I/O dependencies** — they are pure state machines. That is also
what makes them compilable to WASM unchanged.

## Determinism

A run is a pure function of `(scenario, seed)`. See
[determinism.md](determinism.md) for the exact guarantees, enforcement
mechanisms (clippy `disallowed-types`/`disallowed-methods`, virtual clock,
seeded RNG substreams, sequence-number tie-breaks), and the scope of the
byte-identical claim.

## Fixed-point arithmetic

No floating point exists in the matching or accounting path. Prices are `i64`
tick counts, quantities `u64` lot counts, cash `i128` at 1e-8 scale; instrument
constructors validate that `tick_size × lot_size` is exactly representable so
every notional/fee computation is exact integer arithmetic with a single,
documented, adverse-to-strategy rounding rule at the fee boundary. `f64` is
allowed only inside strategy math and the metrics/statistics module.

## Backtest modes (added over the program)

1. **Interactive synthetic** — our engine matches; order flow (Hawkes /
   multi-agent populations) reacts endogenously to your orders.
2. **Non-interactive historical** (Stage 3) — exact L2 delta replay of recorded
   data; your order cannot perturb history, so fills are estimated with an
   explicit family of queue-position models and reported as a sensitivity band.

Each mode's bias profile is documented where it is implemented; neither is
presented as "the truth".

## Verification ladder

property tests (proptest, every push, both OSes) → bounded proofs (Kani, CI on
Linux, same invariant predicates via a shared testkit) → differential testing
(BTree book vs bounded ArrayBook vs ladder book) → mutation testing
(cargo-mutants, scheduled) → deterministic golden-hash runs (BLAKE3 of the
event stream, committed and re-verified).
