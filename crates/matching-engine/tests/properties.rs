//! The property suite: randomized operation sequences checked against an
//! event-stream oracle.
//!
//! The oracle rebuilds the entire book state purely from emitted events (the
//! event-sourcing losslessness claim, P13) while checking, per event:
//! price-time priority (P8), fills at maker price within the taker's limit
//! (P7), no phantom fills (P6), STP (P12), FOK atomicity / IOC-never-rests
//! (P10), modify semantics (P11), and sequencing (P15). After every request
//! the engine's snapshot must equal the oracle's mirror exactly and the
//! book's structural invariants (P1–P4, P9) must hold; at the end, volume
//! conservation (P5/P17) must close order by order.

#![allow(clippy::too_many_lines)]

use std::collections::{BTreeMap, VecDeque};

use matching_engine::{
    Book, BookSnapshot, EngineConfig, MatchingEngine, RestingOrder, SelfMatchPolicy,
    testkit::Harness,
};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use sim_core::ops::{Domain, OpTemplate};
use sim_core::testkit::op_sequence;
use sim_core::{
    CancelReason, EngineEvent, EventRecord, OrderKind, Price, Side, StreamHasher, SymbolId,
    TimeInForce, VecSink,
};

const SYM: SymbolId = SymbolId::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MirrorOrder {
    id: u64,
    owner: u16,
    remaining: u64,
}

#[derive(Debug, Clone, Copy)]
struct Meta {
    owner: u16,
    side: Side,
    kind: OrderKind,
    tif: TimeInForce,
    qty: u64,
}

#[derive(Debug, Default)]
struct Oracle {
    bids: BTreeMap<i64, VecDeque<MirrorOrder>>,
    asks: BTreeMap<i64, VecDeque<MirrorOrder>>,
    /// id → (side, price) for resting orders.
    index: BTreeMap<u64, (Side, i64)>,
    meta: BTreeMap<u64, Meta>,
    outstanding: BTreeMap<u64, u64>,
    filled: BTreeMap<u64, u64>,
    next_seq: u64,
    last_ts: u64,
}

impl Oracle {
    fn ladder(&mut self, side: Side) -> &mut BTreeMap<i64, VecDeque<MirrorOrder>> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    fn best(&self, side: Side) -> Option<i64> {
        match side {
            Side::Buy => self.bids.keys().next_back().copied(),
            Side::Sell => self.asks.keys().next().copied(),
        }
    }

    fn remove_resting(&mut self, id: u64) -> Result<MirrorOrder, String> {
        let (side, price) = *self
            .index
            .get(&id)
            .ok_or_else(|| format!("order {id} not resting in mirror"))?;
        let ladder = self.ladder(side);
        let level = ladder
            .get_mut(&price)
            .ok_or_else(|| format!("mirror level {price} missing for {id}"))?;
        let pos = level
            .iter()
            .position(|o| o.id == id)
            .ok_or_else(|| format!("order {id} not in mirror level {price}"))?;
        let order = level.remove(pos).expect("position exists");
        if level.is_empty() {
            ladder.remove(&price);
        }
        self.index.remove(&id);
        Ok(order)
    }

    fn on_record(&mut self, r: &EventRecord, policy: SelfMatchPolicy) -> Result<(), String> {
        // P15: strict sequencing, non-decreasing time.
        if r.seq.value() != self.next_seq {
            return Err(format!(
                "seq {} != expected {}",
                r.seq.value(),
                self.next_seq
            ));
        }
        self.next_seq += 1;
        if r.ts.nanos() < self.last_ts {
            return Err("timestamps decreased".into());
        }
        self.last_ts = r.ts.nanos();

        match r.event {
            EngineEvent::OrderAccepted {
                id,
                owner,
                side,
                kind,
                qty,
                tif,
            } => {
                let id = id.value();
                if self.meta.contains_key(&id) {
                    return Err(format!("order {id} accepted twice"));
                }
                self.meta.insert(
                    id,
                    Meta {
                        owner: owner.value(),
                        side,
                        kind,
                        tif,
                        qty: qty.lots(),
                    },
                );
                self.outstanding.insert(id, qty.lots());
            }
            EngineEvent::OrderRejected { .. } => {}
            EngineEvent::Fill {
                maker_id,
                taker_id,
                maker_owner,
                taker_owner,
                price,
                qty,
                taker_side,
                maker_remaining,
                taker_remaining,
                ..
            } => {
                let (maker_id, taker_id) = (maker_id.value(), taker_id.value());
                let (price_t, qty_l) = (price.ticks(), qty.lots());
                if qty_l == 0 {
                    return Err("zero-qty fill".into());
                }
                if maker_id == taker_id {
                    return Err("self-referential fill".into());
                }
                // P12: no self-fills under prevention policies.
                if policy != SelfMatchPolicy::Allow && maker_owner == taker_owner {
                    return Err(format!("self-match fill under {policy:?}"));
                }
                // P8 (price priority): the maker must sit at the best level
                // of the maker side.
                let maker_side = taker_side.opposite();
                if self.best(maker_side) != Some(price_t) {
                    return Err(format!(
                        "fill at {price_t} but best {maker_side:?} is {:?}",
                        self.best(maker_side)
                    ));
                }
                // P8 (time priority) + P6 (no phantom fills), inside a scope
                // so the level borrow ends before ladder/index cleanup.
                let (maker_gone, level_empty) = {
                    let level = self
                        .ladder(maker_side)
                        .get_mut(&price_t)
                        .ok_or_else(|| format!("no mirror level at {price_t}"))?;
                    let front = level.front_mut().ok_or("empty mirror level")?;
                    if front.id != maker_id {
                        return Err(format!(
                            "fill against {maker_id} but front of level is {}",
                            front.id
                        ));
                    }
                    if front.owner != maker_owner.value() {
                        return Err("maker owner mismatch".into());
                    }
                    if qty_l > front.remaining {
                        return Err("fill exceeds maker remaining".into());
                    }
                    front.remaining -= qty_l;
                    if front.remaining != maker_remaining.lots() {
                        return Err("maker_remaining mismatch".into());
                    }
                    let gone = front.remaining == 0;
                    if gone {
                        level.pop_front();
                    }
                    (gone, level.is_empty())
                };
                if maker_gone {
                    if level_empty {
                        self.ladder(maker_side).remove(&price_t);
                    }
                    self.index.remove(&maker_id);
                }
                // P7: taker limit satisfied at the maker's price.
                let taker_meta = *self
                    .meta
                    .get(&taker_id)
                    .ok_or_else(|| format!("fill for unknown taker {taker_id}"))?;
                if taker_meta.owner != taker_owner.value() {
                    return Err("taker owner mismatch".into());
                }
                if let OrderKind::Limit(l) = taker_meta.kind {
                    let ok = match taker_side {
                        Side::Buy => price_t <= l.ticks(),
                        Side::Sell => price_t >= l.ticks(),
                    };
                    if !ok {
                        return Err(format!("fill at {price_t} violates taker limit {l}"));
                    }
                }
                // Ledger updates for both parties.
                for (id, expect_after) in
                    [(maker_id, None), (taker_id, Some(taker_remaining.lots()))]
                {
                    let out = self
                        .outstanding
                        .get_mut(&id)
                        .ok_or_else(|| format!("fill for unledgered {id}"))?;
                    *out = out
                        .checked_sub(qty_l)
                        .ok_or_else(|| format!("outstanding underflow for {id}"))?;
                    if let Some(expected) = expect_after
                        && *out != expected
                    {
                        return Err(format!("taker_remaining {expected} != ledger {out}"));
                    }
                    *self.filled.entry(id).or_insert(0) += qty_l;
                }
            }
            EngineEvent::OrderRested {
                id,
                price,
                remaining,
            } => {
                let id = id.value();
                let Some(&Meta {
                    owner,
                    side,
                    kind,
                    tif,
                    ..
                }) = self.meta.get(&id)
                else {
                    return Err(format!("rest of unknown order {id}"));
                };
                // P10: only GTC limit orders rest.
                if tif != TimeInForce::Gtc {
                    return Err(format!("{tif:?} order rested"));
                }
                if matches!(kind, OrderKind::Market) {
                    return Err("market order rested".into());
                }
                if self.index.contains_key(&id) {
                    return Err(format!("order {id} rested twice"));
                }
                if self.outstanding.get(&id) != Some(&remaining.lots()) {
                    return Err("rested remaining != ledger outstanding".into());
                }
                self.ladder(side)
                    .entry(price.ticks())
                    .or_default()
                    .push_back(MirrorOrder {
                        id,
                        owner,
                        remaining: remaining.lots(),
                    });
                self.index.insert(id, (side, price.ticks()));
                // P2: never crossed after a rest.
                if let (Some(b), Some(a)) = (self.best(Side::Buy), self.best(Side::Sell))
                    && b >= a
                {
                    return Err(format!("mirror crossed after rest: {b} >= {a}"));
                }
            }
            EngineEvent::OrderCancelled {
                id,
                remaining,
                reason,
            } => {
                let id = id.value();
                if self.index.contains_key(&id) {
                    // Resting cancel: User or SelfMatch(CancelResting) only.
                    if !matches!(reason, CancelReason::User | CancelReason::SelfMatch) {
                        return Err(format!("resting order cancelled with {reason:?}"));
                    }
                    let order = self.remove_resting(id)?;
                    if order.remaining != remaining.lots() {
                        return Err("cancel remaining != mirror remaining".into());
                    }
                } else {
                    // Incoming-remainder cancel.
                    let meta = *self
                        .meta
                        .get(&id)
                        .ok_or_else(|| format!("cancel of unknown {id}"))?;
                    match reason {
                        CancelReason::User => {
                            return Err("user cancel of non-resting order emitted".into());
                        }
                        CancelReason::FokKill => {
                            if meta.tif != TimeInForce::Fok {
                                return Err("FokKill on non-FOK order".into());
                            }
                            // P10: atomicity — zero fills before a kill.
                            if self.filled.get(&id).copied().unwrap_or(0) != 0 {
                                return Err("FOK killed after partial fill".into());
                            }
                            if remaining.lots() != meta.qty {
                                return Err("FOK kill remaining != full qty".into());
                            }
                        }
                        CancelReason::NoLiquidity => {
                            if !matches!(meta.kind, OrderKind::Market) {
                                return Err("NoLiquidity on limit order".into());
                            }
                        }
                        CancelReason::IocRemainder => {
                            if meta.tif != TimeInForce::Ioc {
                                return Err("IocRemainder on non-IOC order".into());
                            }
                        }
                        CancelReason::SelfMatch => {} // cancelled aggressor remainder
                    }
                }
                let out = self
                    .outstanding
                    .get_mut(&id)
                    .ok_or_else(|| format!("cancel of unledgered {id}"))?;
                if *out != remaining.lots() {
                    return Err(format!(
                        "cancel remaining {} != outstanding {out}",
                        remaining.lots()
                    ));
                }
                *out = 0;
            }
            EngineEvent::OrderModified {
                id,
                new_price,
                new_remaining,
                priority_kept,
            } => {
                let id = id.value();
                let (side, price) = *self
                    .index
                    .get(&id)
                    .ok_or_else(|| format!("modify of non-resting {id}"))?;
                let new_price_t = new_price.ticks();
                if new_price_t == price {
                    let level = self.ladder(side).get_mut(&price).ok_or("level missing")?;
                    let pos = level
                        .iter()
                        .position(|o| o.id == id)
                        .ok_or("order missing from level")?;
                    let current = level[pos].remaining;
                    match new_remaining.lots().cmp(&current) {
                        std::cmp::Ordering::Less => {
                            if !priority_kept {
                                return Err("decrease must keep priority".into());
                            }
                            level[pos].remaining = new_remaining.lots();
                        }
                        std::cmp::Ordering::Greater => {
                            if priority_kept {
                                return Err("increase must lose priority".into());
                            }
                            let mut order = level.remove(pos).expect("pos exists");
                            order.remaining = new_remaining.lots();
                            level.push_back(order);
                        }
                        std::cmp::Ordering::Equal => {
                            if !priority_kept {
                                return Err("no-op modify must keep priority".into());
                            }
                        }
                    }
                } else {
                    // Price change: leaves the book; Fill/Rested events place it.
                    if priority_kept {
                        return Err("price change must lose priority".into());
                    }
                    self.remove_resting(id)?;
                    if let Some(meta) = self.meta.get_mut(&id) {
                        meta.kind = OrderKind::Limit(new_price);
                        meta.qty = new_remaining.lots();
                    }
                }
                self.outstanding.insert(id, new_remaining.lots());
            }
        }
        Ok(())
    }
}

type SideView = Vec<(i64, Vec<MirrorOrder>)>;

fn mirror_snapshot(oracle: &Oracle) -> (SideView, SideView) {
    let bids = oracle
        .bids
        .iter()
        .rev()
        .map(|(p, level)| (*p, level.iter().copied().collect()))
        .collect();
    let asks = oracle
        .asks
        .iter()
        .map(|(p, level)| (*p, level.iter().copied().collect()))
        .collect();
    (bids, asks)
}

fn engine_snapshot(snap: &BookSnapshot) -> (SideView, SideView) {
    let convert = |side: &[(Price, Vec<RestingOrder>)]| {
        side.iter()
            .map(|(p, orders)| {
                (
                    p.ticks(),
                    orders
                        .iter()
                        .map(|o| MirrorOrder {
                            id: o.id.value(),
                            owner: o.owner.value(),
                            remaining: o.remaining.lots(),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    (convert(&snap.bids), convert(&snap.asks))
}

fn policy() -> impl Strategy<Value = SelfMatchPolicy> {
    prop_oneof![
        Just(SelfMatchPolicy::Allow),
        Just(SelfMatchPolicy::CancelResting),
        Just(SelfMatchPolicy::CancelAggressor),
    ]
}

fn run_checked(ops: &[OpTemplate], smp: SelfMatchPolicy) -> Result<(), TestCaseError> {
    let config = EngineConfig {
        self_match: smp,
        ..EngineConfig::default()
    };
    let mut harness = Harness::new(MatchingEngine::new_btree(SYM, config));
    let mut oracle = Oracle::default();

    for op in ops {
        let mut sink = VecSink::new();
        harness
            .apply(op, &mut sink)
            .map_err(|e| TestCaseError::fail(format!("engine corruption: {e}")))?;
        for record in &sink.records {
            oracle.on_record(record, smp).map_err(TestCaseError::fail)?;
        }
        // P13-equivalence: engine book == mirror rebuilt from events alone.
        let engine_view = engine_snapshot(&harness.engine.book().snapshot());
        let mirror_view = mirror_snapshot(&oracle);
        prop_assert_eq!(engine_view, mirror_view, "snapshot != event mirror");
        // P1–P4, P9: structural invariants.
        harness
            .engine
            .book()
            .check_invariants()
            .map_err(|e| TestCaseError::fail(format!("invariant violation: {e}")))?;
    }

    // P5/P17: conservation closes for every order ever accepted.
    for (id, out) in &oracle.outstanding {
        let resting = oracle.index.get(id).map_or(0, |(side, price)| {
            let ladder = match side {
                Side::Buy => &oracle.bids,
                Side::Sell => &oracle.asks,
            };
            ladder[price]
                .iter()
                .find(|o| o.id == *id)
                .map_or(0, |o| o.remaining)
        });
        prop_assert_eq!(*out, resting, "conservation broken for order {}", id);
    }
    // P10 (FOK all-or-nothing, end-of-run form).
    for (id, meta) in &oracle.meta {
        if meta.tif == TimeInForce::Fok {
            let filled = oracle.filled.get(id).copied().unwrap_or(0);
            prop_assert!(
                filled == 0 || filled == meta.qty,
                "FOK {} filled partially: {}/{}",
                id,
                filled,
                meta.qty
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    #[test]
    fn engine_holds_all_invariants(
        ops in op_sequence(Domain::PROPTEST, 120),
        smp in policy(),
    ) {
        run_checked(&ops, smp)?;
    }

    /// P14: byte-identical determinism — the same operations produce the
    /// same event stream, twice, hashed.
    #[test]
    fn engine_is_deterministic(
        ops in op_sequence(Domain::PROPTEST, 80),
        smp in policy(),
    ) {
        let hash = |ops: &[OpTemplate]| -> Result<String, TestCaseError> {
            let config = EngineConfig { self_match: smp, ..EngineConfig::default() };
            let mut harness = Harness::new(MatchingEngine::new_btree(SYM, config));
            let mut hasher = StreamHasher::new();
            for op in ops {
                harness
                    .apply(op, &mut hasher)
                    .map_err(|e| TestCaseError::fail(format!("corruption: {e}")))?;
            }
            Ok(hasher.finalize_hex())
        };
        prop_assert_eq!(hash(&ops)?, hash(&ops)?);
    }
}
