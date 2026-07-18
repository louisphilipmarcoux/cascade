//! Golden example-based tests: hand-written scenarios with exact expected
//! event sequences, one per normative rule in the engine documentation.

use matching_engine::{Book as _, EngineConfig, MatchingEngine, SelfMatchPolicy};
use sim_core::{
    CancelReason, EngineEvent, EventSeq, NewOrder, OrderId, OrderKind, OwnerId, Price, Qty,
    RejectReason, Request, SimTime, StreamHasher, SymbolId, TimeInForce, VecSink,
};

const SYM: SymbolId = SymbolId::new(0);

struct Rig {
    engine: MatchingEngine<matching_engine::BTreeBook>,
    seq: EventSeq,
    ts: SimTime,
    sink: VecSink,
}

impl Rig {
    fn new(policy: SelfMatchPolicy) -> Self {
        let config = EngineConfig {
            self_match: policy,
            ..EngineConfig::default()
        };
        Self {
            engine: MatchingEngine::new_btree(SYM, config),
            seq: EventSeq::new(0),
            ts: SimTime::EPOCH,
            sink: VecSink::new(),
        }
    }

    fn submit(
        &mut self,
        id: u64,
        owner: u16,
        side: sim_core::Side,
        kind: OrderKind,
        qty: u64,
        tif: TimeInForce,
    ) {
        self.step(Request::Submit(NewOrder {
            id: OrderId::new(id),
            owner: OwnerId::new(owner),
            symbol: SYM,
            side,
            kind,
            qty: Qty::new(qty),
            tif,
        }));
    }

    fn limit(&mut self, id: u64, owner: u16, side: sim_core::Side, price: i64, qty: u64) {
        self.submit(
            id,
            owner,
            side,
            OrderKind::Limit(Price::new(price)),
            qty,
            TimeInForce::Gtc,
        );
    }

    fn step(&mut self, request: Request) {
        self.ts = SimTime::from_micros(self.ts.nanos() / 1_000 + 1);
        self.engine
            .process(self.ts, &mut self.seq, &mut self.sink, request)
            .expect("engine must not corrupt");
    }

    /// Events emitted so far (drains nothing; use `events_since` for windows).
    fn events(&self) -> Vec<EngineEvent> {
        self.sink.records.iter().map(|r| r.event).collect()
    }

    fn events_from(&self, start: usize) -> Vec<EngineEvent> {
        self.sink.records[start..].iter().map(|r| r.event).collect()
    }

    fn mark(&self) -> usize {
        self.sink.records.len()
    }
}

use sim_core::Side::{Buy, Sell};

#[test]
fn simple_cross_fills_fully_at_maker_price() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 5);
    let mark = rig.mark();
    rig.limit(1, 2, Buy, 101, 5); // taker limit 101, maker at 100 → executes at 100

    let events = rig.events_from(mark);
    assert!(matches!(events[0], EngineEvent::OrderAccepted { .. }));
    match events[1] {
        EngineEvent::Fill {
            price,
            qty,
            maker_remaining,
            taker_remaining,
            taker_side,
            ..
        } => {
            assert_eq!(price, Price::new(100));
            assert_eq!(qty, Qty::new(5));
            assert_eq!(maker_remaining, Qty::new(0));
            assert_eq!(taker_remaining, Qty::new(0));
            assert_eq!(taker_side, Buy);
        }
        ref other => panic!("expected fill, got {other:?}"),
    }
    assert_eq!(events.len(), 2, "fully filled: no rest/cancel event");
    assert_eq!(rig.engine.book().snapshot().order_count(), 0);
}

#[test]
fn partial_fill_rests_remainder() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 3);
    let mark = rig.mark();
    rig.limit(1, 2, Buy, 100, 5);

    let events = rig.events_from(mark);
    assert!(matches!(events[1], EngineEvent::Fill { qty, .. } if qty == Qty::new(3)));
    match events[2] {
        EngineEvent::OrderRested {
            price, remaining, ..
        } => {
            assert_eq!(price, Price::new(100));
            assert_eq!(remaining, Qty::new(2));
        }
        ref other => panic!("expected rest, got {other:?}"),
    }
    let snap = rig.engine.book().snapshot();
    assert_eq!(snap.best_bid(), Some(Price::new(100)));
    assert_eq!(snap.best_ask(), None);
}

#[test]
fn fifo_within_level_fills_oldest_first() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 4); // A
    rig.limit(1, 2, Sell, 100, 4); // B (behind A)
    let mark = rig.mark();
    rig.limit(2, 0, Buy, 100, 6);

    let events = rig.events_from(mark);
    match (&events[1], &events[2]) {
        (
            EngineEvent::Fill {
                maker_id: m1,
                qty: q1,
                ..
            },
            EngineEvent::Fill {
                maker_id: m2,
                qty: q2,
                maker_remaining,
                ..
            },
        ) => {
            assert_eq!(
                (*m1, *q1),
                (OrderId::new(0), Qty::new(4)),
                "A fully filled first"
            );
            assert_eq!(
                (*m2, *q2),
                (OrderId::new(1), Qty::new(2)),
                "then B partially"
            );
            assert_eq!(*maker_remaining, Qty::new(2));
        }
        other => panic!("expected two fills, got {other:?}"),
    }
}

#[test]
fn sweeps_price_levels_best_first_at_maker_prices() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 2);
    rig.limit(1, 1, Sell, 99, 2); // better ask, later arrival
    let mark = rig.mark();
    rig.limit(2, 2, Buy, 101, 4);

    let events = rig.events_from(mark);
    match (&events[1], &events[2]) {
        (EngineEvent::Fill { price: p1, .. }, EngineEvent::Fill { price: p2, .. }) => {
            assert_eq!(*p1, Price::new(99), "best price first");
            assert_eq!(*p2, Price::new(100));
        }
        other => panic!("expected two fills, got {other:?}"),
    }
}

#[test]
fn market_order_sweeps_then_cancels_no_liquidity() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 5);
    let mark = rig.mark();
    rig.submit(1, 2, Buy, OrderKind::Market, 8, TimeInForce::Ioc);

    let events = rig.events_from(mark);
    assert!(matches!(events[1], EngineEvent::Fill { qty, .. } if qty == Qty::new(5)));
    assert!(matches!(
        events[2],
        EngineEvent::OrderCancelled { remaining, reason: CancelReason::NoLiquidity, .. }
            if remaining == Qty::new(3)
    ));
}

#[test]
fn ioc_remainder_never_rests() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 2);
    let mark = rig.mark();
    rig.submit(
        1,
        2,
        Buy,
        OrderKind::Limit(Price::new(100)),
        5,
        TimeInForce::Ioc,
    );

    let events = rig.events_from(mark);
    assert!(matches!(
        events[2],
        EngineEvent::OrderCancelled { remaining, reason: CancelReason::IocRemainder, .. }
            if remaining == Qty::new(3)
    ));
    assert_eq!(rig.engine.book().snapshot().best_bid(), None);
}

#[test]
fn fok_kills_atomically_or_fills_fully() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 3);
    let mark = rig.mark();
    // Needs 5, only 3 available in limit → kill with zero fills.
    rig.submit(
        1,
        2,
        Buy,
        OrderKind::Limit(Price::new(100)),
        5,
        TimeInForce::Fok,
    );
    let events = rig.events_from(mark);
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[1],
        EngineEvent::OrderCancelled { remaining, reason: CancelReason::FokKill, .. }
            if remaining == Qty::new(5)
    ));
    // Book untouched.
    assert_eq!(
        rig.engine.book().snapshot().best_ask(),
        Some(Price::new(100))
    );

    // Feasible FOK fills fully.
    let mark = rig.mark();
    rig.submit(
        2,
        2,
        Buy,
        OrderKind::Limit(Price::new(100)),
        3,
        TimeInForce::Fok,
    );
    let events = rig.events_from(mark);
    assert!(
        matches!(events[1], EngineEvent::Fill { qty, taker_remaining, .. }
        if qty == Qty::new(3) && taker_remaining == Qty::new(0))
    );
}

#[test]
fn stp_cancel_resting_removes_own_order_and_continues() {
    let mut rig = Rig::new(SelfMatchPolicy::CancelResting);
    rig.limit(0, 7, Sell, 100, 2); // own resting
    rig.limit(1, 1, Sell, 100, 2); // other's, behind
    let mark = rig.mark();
    rig.limit(2, 7, Buy, 100, 2); // same owner aggresses

    let events = rig.events_from(mark);
    assert!(matches!(
        events[1],
        EngineEvent::OrderCancelled { id, reason: CancelReason::SelfMatch, .. }
            if id == OrderId::new(0)
    ));
    assert!(matches!(events[2], EngineEvent::Fill { maker_id, .. } if maker_id == OrderId::new(1)));
}

#[test]
fn stp_cancel_aggressor_stops_at_own_order() {
    let mut rig = Rig::new(SelfMatchPolicy::CancelAggressor);
    rig.limit(0, 1, Sell, 99, 1); // other's, better price
    rig.limit(1, 7, Sell, 100, 2); // own
    let mark = rig.mark();
    rig.limit(2, 7, Buy, 101, 5);

    let events = rig.events_from(mark);
    assert!(matches!(events[1], EngineEvent::Fill { maker_id, .. } if maker_id == OrderId::new(0)));
    assert!(matches!(
        events[2],
        EngineEvent::OrderCancelled { id, remaining, reason: CancelReason::SelfMatch }
            if id == OrderId::new(2) && remaining == Qty::new(4)
    ));
    // Own resting order untouched.
    assert!(rig.engine.book().get(OrderId::new(1)).is_some());
}

#[test]
fn modify_decrease_keeps_priority() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 5); // A
    rig.limit(1, 2, Sell, 100, 5); // B
    rig.step(Request::Modify {
        id: OrderId::new(0),
        new_price: None,
        new_remaining: Qty::new(2),
    });
    let mark = rig.mark();
    rig.limit(2, 0, Buy, 100, 1);

    let events = rig.events_from(mark);
    assert!(
        matches!(events[1], EngineEvent::Fill { maker_id, .. } if maker_id == OrderId::new(0)),
        "A still fills first after qty decrease"
    );
}

#[test]
fn modify_increase_loses_priority() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Sell, 100, 5); // A
    rig.limit(1, 2, Sell, 100, 5); // B
    let mark = rig.mark();
    rig.step(Request::Modify {
        id: OrderId::new(0),
        new_price: None,
        new_remaining: Qty::new(6),
    });
    assert!(matches!(
        rig.events_from(mark)[0],
        EngineEvent::OrderModified {
            priority_kept: false,
            ..
        }
    ));
    let mark = rig.mark();
    rig.limit(2, 0, Buy, 100, 1);
    assert!(
        matches!(rig.events_from(mark)[1], EngineEvent::Fill { maker_id, .. } if maker_id == OrderId::new(1)),
        "B fills first after A's qty increase"
    );
}

#[test]
fn modify_price_change_rematches_and_can_fill() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Buy, 99, 3); // resting bid
    rig.limit(1, 2, Sell, 100, 2); // ask
    let mark = rig.mark();
    rig.step(Request::Modify {
        id: OrderId::new(0),
        new_price: Some(Price::new(101)),
        new_remaining: Qty::new(3),
    });

    let events = rig.events_from(mark);
    assert!(matches!(
        events[0],
        EngineEvent::OrderModified { priority_kept: false, new_price, .. }
            if new_price == Price::new(101)
    ));
    assert!(matches!(events[1], EngineEvent::Fill { price, qty, .. }
        if price == Price::new(100) && qty == Qty::new(2)));
    assert!(
        matches!(events[2], EngineEvent::OrderRested { price, remaining, .. }
        if price == Price::new(101) && remaining == Qty::new(1))
    );
}

#[test]
fn validation_rejects() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    // Zero qty.
    rig.submit(
        0,
        1,
        Buy,
        OrderKind::Limit(Price::new(100)),
        0,
        TimeInForce::Gtc,
    );
    // Non-positive price.
    rig.submit(
        1,
        1,
        Buy,
        OrderKind::Limit(Price::new(0)),
        1,
        TimeInForce::Gtc,
    );
    // Market + GTC.
    rig.submit(2, 1, Buy, OrderKind::Market, 1, TimeInForce::Gtc);
    // Valid order, then a duplicate (non-increasing) id.
    rig.limit(5, 1, Buy, 50, 1);
    rig.submit(
        5,
        1,
        Buy,
        OrderKind::Limit(Price::new(50)),
        1,
        TimeInForce::Gtc,
    );
    rig.submit(
        3,
        1,
        Buy,
        OrderKind::Limit(Price::new(50)),
        1,
        TimeInForce::Gtc,
    );
    // Unknown cancel and zero-remaining modify.
    rig.step(Request::Cancel {
        id: OrderId::new(99),
    });
    rig.step(Request::Modify {
        id: OrderId::new(5),
        new_price: None,
        new_remaining: Qty::new(0),
    });

    let reasons: Vec<RejectReason> = rig
        .events()
        .iter()
        .filter_map(|e| match e {
            EngineEvent::OrderRejected { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![
            RejectReason::ZeroQty,
            RejectReason::NonPositivePrice,
            RejectReason::MarketGtc,
            RejectReason::DuplicateOrderId,
            RejectReason::DuplicateOrderId,
            RejectReason::UnknownOrder,
            RejectReason::ZeroModifyRemaining,
        ]
    );
}

#[test]
fn cancel_removes_and_second_cancel_rejects() {
    let mut rig = Rig::new(SelfMatchPolicy::Allow);
    rig.limit(0, 1, Buy, 50, 3);
    let mark = rig.mark();
    rig.step(Request::Cancel {
        id: OrderId::new(0),
    });
    rig.step(Request::Cancel {
        id: OrderId::new(0),
    });

    let events = rig.events_from(mark);
    assert!(matches!(
        events[0],
        EngineEvent::OrderCancelled { remaining, reason: CancelReason::User, .. }
            if remaining == Qty::new(3)
    ));
    assert!(matches!(
        events[1],
        EngineEvent::OrderRejected {
            reason: RejectReason::UnknownOrder,
            ..
        }
    ));
}

/// Deterministic golden stream: this exact scenario must hash to this exact
/// value on every platform and every run. If an intentional semantics change
/// alters the stream, regenerate the constant in the same commit and say so.
#[test]
fn golden_stream_hash_is_stable() {
    let mut rig = Rig::new(SelfMatchPolicy::CancelResting);
    rig.limit(0, 1, Sell, 100, 4);
    rig.limit(1, 2, Sell, 100, 4);
    rig.limit(2, 1, Buy, 99, 3);
    rig.submit(
        3,
        3,
        Buy,
        OrderKind::Limit(Price::new(100)),
        6,
        TimeInForce::Gtc,
    );
    rig.submit(4, 2, Buy, OrderKind::Market, 5, TimeInForce::Ioc);
    rig.step(Request::Modify {
        id: OrderId::new(2),
        new_price: Some(Price::new(101)),
        new_remaining: Qty::new(3),
    });
    rig.step(Request::Cancel {
        id: OrderId::new(2),
    });

    let mut hasher = StreamHasher::new();
    for record in &rig.sink.records {
        use sim_core::EventSink as _;
        hasher.emit(*record);
    }
    assert_eq!(
        hasher.finalize_hex(),
        "a7a1c4122cd09cf797f7d5c699b25883992c1570956ab3aafcfc193f2baec0aa",
        "event stream changed; if intentional, regenerate this constant"
    );
}
