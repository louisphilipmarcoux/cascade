# Determinism: guarantees, enforcement, scope

## The claim

Running the same binary, on the same platform, with the same scenario file and
seed produces a **byte-identical event stream** (and therefore identical
reports). Every run prints the BLAKE3 hash of its serialized event stream;
`quant-sim run --verify-hash <hex>` re-runs and compares.

## What enforces it

- **Virtual time only.** The simulation clock advances by popping a
  discrete-event queue; `SystemTime::now`/`Instant::now` are banned by clippy
  `disallowed-methods` (CLI report metadata opts out explicitly at the call
  site).
- **Single-threaded, no async.** The DES is fully synchronous by design — a
  simplicity claim, not a limitation.
- **No nondeterministic iteration.** `std::collections::HashMap`/`HashSet` are
  banned by clippy `disallowed-types`. Ordered iteration uses `BTreeMap`;
  point-lookup-only maps use `FxHashMap` and are never iterated.
- **Seeded randomness.** One master seed; every component draws from its own
  ChaCha12 substream, so adding a strategy does not perturb the background
  flow's stream (this is what makes paired-seed A/B comparisons valid).
- **Total order of events.** The event queue breaks timestamp ties with a
  monotonic sequence number assigned at push.
- **LF everywhere.** All emitted artifacts are written with explicit `\n` so
  file hashes do not depend on the platform's line-ending convention.

## Scope and honesty

- Byte-identical: **same binary, same platform** — guaranteed and tested.
- Cross-platform (Linux vs Windows): the integer-only engine/flow path is
  expected to match; strategy code uses `f64` (libm functions may differ per
  platform). A CI job runs the same scenario on both OSes and **reports**
  whether the hashes match rather than assuming. Portable transcendentals
  (`libm` crate) are used in simulation-observable float paths to maximize the
  chance they do.
- Kani proofs run on Linux only (upstream limitation); the identical invariant
  predicates run under proptest on both platforms.
