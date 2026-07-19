# matching-engine

Price-time-priority limit order book and matching engine. The matching
algorithm is written once, generic over the `Book` trait; the production
implementation is `BTreeBook`.

## Design decisions (and rejected alternatives)

- **`BTreeMap` price ladder + slab-backed intrusive FIFO levels + id index.**
  O(log P) to open a price level, O(1) enqueue, O(1) cancel/modify by id,
  O(1) best-price, fully deterministic iteration, zero `unsafe`. A contiguous
  price-band ladder (`Vec` with offset indexing) is implemented at M9 and
  *benchmarked against* this book rather than assumed faster; a lock-free
  variant is a named seam (Stage 7), deliberately built only after the safe
  version is fully proven.
- **The engine's only output is the event stream.** L2 book deltas are a
  projection rebuilt from order lifecycle events; losslessness is enforced by
  P13 below, not by convention.
- **Self-match prevention is in v1** (`Allow` / `CancelResting` (default) /
  `CancelAggressor`): a backtest where a strategy fills itself corrupts PnL.
  The FOK feasibility count is STP-mode-aware (`All` / `Skip` / `StopAt`) —
  under `CancelAggressor`, liquidity behind your own resting order is
  unreachable and must not be counted; getting this wrong silently breaks FOK
  atomicity, which is why the mode is an explicit type.
- **Ids must be strictly increasing across accepted submissions** (they come
  from one monotonic generator per run), so duplicate detection is O(1) with
  no unbounded id-set memory.

## Normative semantics (tests are written from this list)

Priority: strict price-time (FIFO by acceptance within a level). Execution
price is always the **maker's** resting price. Event grammar per submission:
`OrderAccepted` → `Fill`* → exactly one of `OrderRested` / `OrderCancelled` /
nothing (fully filled). Market orders sweep and never rest; IOC remainders
cancel; FOK pre-checks feasibility atomically (zero fills on kill). Modify:
price change = cancel+replace under the same id (loses priority, re-enters
matching); qty decrease keeps priority; qty increase moves to the tail of the
level.

## Invariants ↔ tests ↔ proofs

| # | Invariant | proptest / golden | Kani |
|---|-----------|-------------------|------|
| P1 | Sides strictly sorted, no empty levels | `check_invariants` after every op | K2 (via check) |
| P2 | Book never crossed at rest | oracle + `check_invariants` | K2 |
| P3 | Level totals == Σ member remainings | `check_invariants` | K2 (via check) |
| P4 | Slab/index/list mutual consistency | `check_invariants` | — |
| P5 | Volume conservation (per order + global) | oracle end-of-run ledger | K1 |
| P6 | No phantom fills | oracle per-fill | K3 (implicit: Kani overflow/panic checks) |
| P7 | Fill at maker price within taker limit | oracle per-fill | — |
| P8 | Price-time priority honored | oracle per-fill (best level + front) | — |
| P9 | No zero/negative quantities | `check_invariants` + oracle | K3 (implicit: Kani overflow/panic checks) |
| P10 | IOC/Market never rest; FOK atomic | oracle + `fok_kills_atomically…` | K4 |
| P11 | Modify priority rules | oracle + golden modify tests | — |
| P12 | No self-fills under STP ≠ Allow | oracle per-fill | — |
| P13 | Event log losslessly rebuilds the book | snapshot ≡ event mirror, every op | — |
| P14 | Determinism (same ops ⇒ same stream) | `engine_is_deterministic` + golden hash | — |
| P15 | EventSeq strictly +1; ts non-decreasing | oracle per-record | — |
| P16 | Cancelled id is gone | golden `cancel_removes…` | — |
| P17 | Conservation through modify chains | oracle ledger (modify re-declares) | — |
| P18 | Serde/bincode round-trip | sim-core event tests | — |

The proptest oracle (`tests/properties.rs`) rebuilds the whole book purely
from emitted events and compares it to the engine's snapshot after **every**
operation — the strongest form of P13 — while checking P5–P12/P15 per event.
Golden scenarios (`tests/scenarios.rs`) pin one exact expected event sequence
per normative rule, plus a committed BLAKE3 golden stream hash.

## Extension seams

Price collars/halts; iceberg orders; the ladder book (M9 benchmark); the
lock-free variant (Stage 7); full L2 delta replay (Stage 3).
