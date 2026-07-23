//! Kani bounded proof harnesses (compiled only under `cargo kani`).
//!
//! These prove the invariants for **all** possible bounded inputs — every
//! order side/kind/qty/price/TIF/owner combination in the Kani domain, every
//! self-match policy, every interleaving of ≤ `OPS` operations — not just the
//! ones proptest happened to sample. The engine runs against
//! [`ArrayBook`], and `tests/differential.rs` pins `ArrayBook ≡ BTreeBook`,
//! which is how this confidence transfers to the production book.
//!
//! Shared vocabulary: the same [`OpTemplate`] model and [`Harness`] applier
//! the proptest suites use (`sim_core::ops`, `crate::testkit`).
//!
//! Kani additionally checks every harness for arithmetic overflow, panics
//! and unreachable code on all paths (K3 is implicit in K1/K2/K4).

use sim_core::ops::{Domain, OpTemplate, SubmitSpec};
use sim_core::{
    EngineEvent, OrderKind, Price, Qty, Side, SymbolId, TimeInForce, VecSink,
    price::{Cash, E8},
};

use crate::book::Book;
use crate::book::array::ArrayBook;
use crate::engine::{EngineConfig, MatchingEngine, SelfMatchPolicy};
use crate::testkit::Harness;

const DOM: Domain = Domain::KANI;
/// Operations per proof run. Every submission can rest, so CAP ≥ OPS.
const OPS: usize = 3;
const CAP: usize = 4;

fn any_side() -> Side {
    if kani::any() { Side::Buy } else { Side::Sell }
}

fn any_policy() -> SelfMatchPolicy {
    match kani::any::<u8>() % 3 {
        0 => SelfMatchPolicy::Allow,
        1 => SelfMatchPolicy::CancelResting,
        _ => SelfMatchPolicy::CancelAggressor,
    }
}

fn any_spec() -> SubmitSpec {
    let kind = if kani::any() {
        OrderKind::Limit(Price::new(kani::any()))
    } else {
        OrderKind::Market
    };
    let tif = match kani::any::<u8>() % 3 {
        0 => TimeInForce::Gtc,
        1 => TimeInForce::Ioc,
        _ => TimeInForce::Fok,
    };
    let spec = SubmitSpec {
        side: any_side(),
        kind,
        qty: Qty::new(kani::any()),
        tif,
        owner_index: kani::any(),
    };
    kani::assume(spec.is_valid(&DOM));
    spec
}

fn any_op() -> OpTemplate {
    match kani::any::<u8>() % 3 {
        0 => OpTemplate::Submit(any_spec()),
        1 => OpTemplate::Cancel {
            target: kani::any::<usize>() % OPS,
        },
        _ => {
            let new_price = if kani::any() {
                let ticks: i64 = kani::any();
                kani::assume(ticks >= 1 && ticks <= DOM.max_price_ticks);
                Some(Price::new(ticks))
            } else {
                None
            };
            let lots: u64 = kani::any();
            kani::assume(lots >= 1 && lots <= DOM.max_qty_lots);
            OpTemplate::Modify {
                target: kani::any::<usize>() % OPS,
                new_price,
                new_remaining: Qty::new(lots),
            }
        }
    }
}

/// Per-order ledger rebuilt from the event stream (ids are harness-assigned,
/// dense in `0..OPS`, so fixed arrays suffice).
#[derive(Default)]
struct Ledger {
    accepted: [bool; OPS],
    qty: [u64; OPS],
    outstanding: [u64; OPS],
    filled: [u64; OPS],
    is_fok: [bool; OPS],
}

impl Ledger {
    fn absorb(&mut self, sink: &VecSink) {
        for record in &sink.records {
            match record.event {
                EngineEvent::OrderAccepted { id, qty, tif, .. } => {
                    let id = id.value() as usize;
                    self.accepted[id] = true;
                    self.qty[id] = qty.lots();
                    self.outstanding[id] = qty.lots();
                    self.is_fok[id] = tif == TimeInForce::Fok;
                }
                EngineEvent::Fill {
                    maker_id,
                    taker_id,
                    qty,
                    ..
                } => {
                    for id in [maker_id.value() as usize, taker_id.value() as usize] {
                        self.outstanding[id] -= qty.lots();
                        self.filled[id] += qty.lots();
                    }
                }
                EngineEvent::OrderCancelled { id, remaining, .. } => {
                    let id = id.value() as usize;
                    assert!(
                        self.outstanding[id] == remaining.lots(),
                        "cancel remaining must equal outstanding"
                    );
                    self.outstanding[id] = 0;
                }
                EngineEvent::OrderModified {
                    id, new_remaining, ..
                } => {
                    self.outstanding[id.value() as usize] = new_remaining.lots();
                }
                EngineEvent::OrderRested { id, remaining, .. } => {
                    assert!(
                        self.outstanding[id.value() as usize] == remaining.lots(),
                        "rested remaining must equal outstanding"
                    );
                }
                EngineEvent::OrderRejected { .. } => {}
            }
        }
    }
}

fn fresh_harness() -> Harness<ArrayBook<CAP>> {
    let config = EngineConfig {
        self_match: any_policy(),
        max_order_qty: Qty::new(DOM.max_qty_lots),
    };
    Harness::new(MatchingEngine::new(
        SymbolId::new(0),
        config,
        ArrayBook::new(),
    ))
}

/// K1 — volume conservation: for every order, what was accepted equals what
/// filled plus what was cancelled plus what still rests, across arbitrary
/// bounded operation sequences (including modify chains, which re-declare
/// the outstanding quantity).
#[kani::proof]
#[kani::unwind(24)]
fn k1_volume_conservation() {
    let mut harness = fresh_harness();
    let mut sink = VecSink::new();
    for _ in 0..OPS {
        let op = any_op();
        let ok = harness.apply(&op, &mut sink).is_ok();
        assert!(ok, "engine must not corrupt");
    }
    let mut ledger = Ledger::default();
    ledger.absorb(&sink);
    for id in &harness.issued {
        let idx = id.value() as usize;
        if !ledger.accepted[idx] {
            continue;
        }
        let resting = harness
            .engine
            .book()
            .get(*id)
            .map_or(0, |o| o.remaining.lots());
        assert!(
            ledger.outstanding[idx] == resting,
            "conservation: outstanding must equal resting remainder"
        );
    }
}

/// K2 — the book is never crossed after any operation completes, from any
/// reachable bounded state.
#[kani::proof]
#[kani::unwind(24)]
fn k2_never_crossed() {
    let mut harness = fresh_harness();
    let mut sink = VecSink::new();
    for _ in 0..OPS {
        let op = any_op();
        assert!(harness.apply(&op, &mut sink).is_ok());
        let book = harness.engine.book();
        if let (Some(bid), Some(ask)) = (book.best_price(Side::Buy), book.best_price(Side::Sell)) {
            assert!(bid < ask, "book crossed at rest");
        }
        assert!(book.check_invariants().is_ok());
    }
}

/// K4 — FOK atomicity: every FOK order either never fills or fills exactly
/// its full quantity, under all three self-match policies.
#[kani::proof]
#[kani::unwind(24)]
fn k4_fok_atomicity() {
    let mut harness = fresh_harness();
    let mut sink = VecSink::new();
    for _ in 0..OPS {
        let op = any_op();
        assert!(harness.apply(&op, &mut sink).is_ok());
    }
    let mut ledger = Ledger::default();
    ledger.absorb(&sink);
    for idx in 0..OPS {
        if ledger.accepted[idx] && ledger.is_fok[idx] {
            assert!(
                ledger.filled[idx] == 0 || ledger.filled[idx] == ledger.qty[idx],
                "FOK partial fill"
            );
        }
    }
}

/// K5 — fixed-point money arithmetic: for all bounded valid instruments,
/// prices, quantities and rates, notional computation cannot overflow and
/// the single fee-rounding rule stays within one e8 unit of the exact value,
/// always rounding toward +∞ (adverse to the strategy).
///
/// Domain note: the harness deliberately does **not** build an
/// [`sim_core::instrument::Instrument`] — that type carries a heap-allocated
/// `String` name, and a runtime `String` on a proof path makes CBMC unwind
/// `core::slice::memchr` unboundedly (the job never terminates). Instead the
/// notional is computed inline from a representative tick·lot value exactly
/// as `Instrument::notional` does (`ticks · lots · tick_lot_value_e8` in
/// i128), keeping the proof allocation-free. The three values match the
/// BTC-perp-like / equity-like / penny-tick instruments.
/// The symbolic bounds are deliberately narrow (10-bit price/qty, ±10k rate):
/// the rounding property is structural — every sign and remainder path is
/// exercised at small magnitudes — and wide bit-widths only multiply SAT
/// cost. Wide-range coverage is the proptest suite's job (P20); this proof
/// nails the ceil-toward-+∞ arithmetic itself.
#[kani::proof]
#[kani::unwind(4)]
fn k5_fee_arithmetic_bounds() {
    // Quote currency per (tick·lot) at e8 scale, = tick_size_e8·lot_size_e8/1e8
    // for each representative instrument.
    let tick_lot_value_e8: i64 = match kani::any::<u8>() % 3 {
        0 => 10_000,    // tick 0.10, lot 0.001 (BTC-perp-like)
        1 => 1_000_000, // tick 0.01, lot 1.0 (equity-like)
        _ => 1_000_000, // tick 1.00, lot 0.01 (penny-tick)
    };

    let ticks: i64 = kani::any();
    let lots: u64 = kani::any();
    kani::assume(ticks >= 1 && ticks <= 1023);
    kani::assume(lots >= 1 && lots <= 1023);
    // notional = ticks · lots · tick_lot_value_e8 (i128), overflow-free at
    // these bounds (< 1.1e15); this mirrors Instrument::notional exactly.
    let notional_e8 = i128::from(ticks) * i128::from(lots) * i128::from(tick_lot_value_e8);
    let notional = Cash::from_e8(notional_e8);

    let rate_e8: i128 = kani::any();
    kani::assume(rate_e8 >= -10_000 && rate_e8 <= 10_000); // ±1 bp granularity
    let fee = notional.checked_mul_rate_e8_ceil(rate_e8);
    assert!(fee.is_ok(), "bounded fee must not overflow");
    let Ok(fee) = fee else { return };
    let exact_num = notional.e8() * rate_e8; // fee ≈ exact_num / 1e8
    assert!(fee.e8() * E8 >= exact_num, "fee rounds toward +inf");
    assert!(fee.e8() * E8 - exact_num < E8, "fee within one e8 unit");
}

/// K6 — checked tick algebra round-trips when it succeeds.
#[kani::proof]
#[kani::unwind(8)]
fn k6_checked_op_algebra() {
    let a: i64 = kani::any();
    let d: i64 = kani::any();
    kani::assume(d != i64::MIN); // -d must be representable
    let p = Price::new(a);
    if let Ok(q) = p.checked_add_ticks(d)
        && let Ok(back) = q.checked_add_ticks(-d)
    {
        assert!(back == p, "(a+d)-d == a");
    }
    let x: u64 = kani::any();
    let y: u64 = kani::any();
    if let Ok(sum) = Qty::new(x).checked_add(Qty::new(y)) {
        let diff = sum.checked_sub(Qty::new(y));
        assert!(diff.is_ok(), "sum >= y must subtract");
        if let Ok(diff) = diff {
            assert!(diff == Qty::new(x), "(x+y)-y == x");
        }
    }
}
