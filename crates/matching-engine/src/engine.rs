//! The matching algorithm: price-time priority, IOC/FOK, self-match
//! prevention, and modify semantics.
//!
//! Normative event grammar per submission: `OrderAccepted` → zero or more
//! `Fill` → exactly one of `OrderRested` (GTC remainder) /
//! `OrderCancelled` (IOC remainder, FOK kill, exhausted market order,
//! self-match-cancelled aggressor) / nothing (fully filled — the last fill
//! carries `taker_remaining == 0`). Invalid submissions emit a single
//! `OrderRejected`. Every fill executes at the **maker's** resting price.

use serde::{Deserialize, Serialize};
use sim_core::{
    CancelReason, EngineEvent, EventRecord, EventSeq, EventSink, IdGen, NewOrder, OrderKind, Qty,
    RejectReason, Request, SimTime, SymbolId, TimeInForce,
};

use crate::book::{Book, BookError, RestingOrder, StpCount, btree::BTreeBook, crosses};

/// Self-match prevention policy (keyed by `OwnerId`).
///
/// A backtest in which a strategy fills against itself corrupts `PnL`, so the
/// default is `CancelResting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfMatchPolicy {
    /// Own orders match each other (documented-limitation mode).
    Allow,
    /// The resting own order is cancelled; the aggressor keeps matching.
    #[default]
    CancelResting,
    /// The aggressor's remainder is cancelled at the first own resting order.
    CancelAggressor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EngineConfig {
    pub self_match: SelfMatchPolicy,
    pub max_order_qty: Qty,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            self_match: SelfMatchPolicy::default(),
            max_order_qty: Qty::new(1_000_000),
        }
    }
}

/// One instrument's matching engine, generic over the book implementation.
#[derive(Debug)]
pub struct MatchingEngine<B: Book> {
    symbol: SymbolId,
    config: EngineConfig,
    book: B,
    /// Highest order id accepted so far. Submissions must carry strictly
    /// increasing ids (they come from one monotonic [`IdGen`] per run);
    /// anything else is rejected as [`RejectReason::DuplicateOrderId`].
    last_accepted_id: Option<u64>,
    trade_ids: IdGen,
}

impl MatchingEngine<BTreeBook> {
    #[must_use]
    pub fn new_btree(symbol: SymbolId, config: EngineConfig) -> Self {
        Self::new(symbol, config, BTreeBook::new())
    }
}

impl<B: Book> MatchingEngine<B> {
    #[must_use]
    pub fn new(symbol: SymbolId, config: EngineConfig, book: B) -> Self {
        Self {
            symbol,
            config,
            book,
            last_accepted_id: None,
            trade_ids: IdGen::new(),
        }
    }

    #[must_use]
    pub fn book(&self) -> &B {
        &self.book
    }

    #[must_use]
    pub const fn symbol(&self) -> SymbolId {
        self.symbol
    }

    #[must_use]
    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Process one request to completion, emitting events into `sink`.
    ///
    /// `seq` is the run-global event sequence counter (shared across engines
    /// so the whole run has one totally ordered stream). Returns `Err` only
    /// on internal corruption — never for invalid user input, which is
    /// reported as `OrderRejected` events.
    pub fn process(
        &mut self,
        ts: SimTime,
        seq: &mut EventSeq,
        sink: &mut dyn EventSink,
        request: Request,
    ) -> Result<(), BookError> {
        match request {
            Request::Submit(order) => self.submit(ts, seq, sink, order),
            Request::Cancel { id } => {
                match self.book.remove(id) {
                    Ok(resting) => emit(
                        sink,
                        seq,
                        ts,
                        self.symbol,
                        EngineEvent::OrderCancelled {
                            id,
                            remaining: resting.remaining,
                            reason: CancelReason::User,
                        },
                    ),
                    Err(BookError::NotFound(_)) => emit(
                        sink,
                        seq,
                        ts,
                        self.symbol,
                        EngineEvent::OrderRejected {
                            id,
                            reason: RejectReason::UnknownOrder,
                        },
                    ),
                    Err(other) => return Err(other),
                }
                Ok(())
            }
            Request::Modify {
                id,
                new_price,
                new_remaining,
            } => self.modify(ts, seq, sink, id, new_price, new_remaining),
        }
    }

    fn validate(&self, order: &NewOrder) -> Option<RejectReason> {
        if let Some(last) = self.last_accepted_id
            && order.id.value() <= last
        {
            return Some(RejectReason::DuplicateOrderId);
        }
        if order.qty.is_zero() {
            return Some(RejectReason::ZeroQty);
        }
        if let OrderKind::Limit(price) = order.kind
            && !price.is_positive()
        {
            return Some(RejectReason::NonPositivePrice);
        }
        if matches!(order.kind, OrderKind::Market) && order.tif == TimeInForce::Gtc {
            return Some(RejectReason::MarketGtc);
        }
        if order.qty > self.config.max_order_qty {
            return Some(RejectReason::QtyAboveMax);
        }
        None
    }

    fn submit(
        &mut self,
        ts: SimTime,
        seq: &mut EventSeq,
        sink: &mut dyn EventSink,
        order: NewOrder,
    ) -> Result<(), BookError> {
        if let Some(reason) = self.validate(&order) {
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                EngineEvent::OrderRejected {
                    id: order.id,
                    reason,
                },
            );
            return Ok(());
        }
        self.last_accepted_id = Some(order.id.value());
        emit(
            sink,
            seq,
            ts,
            self.symbol,
            EngineEvent::OrderAccepted {
                id: order.id,
                owner: order.owner,
                side: order.side,
                kind: order.kind,
                qty: order.qty,
                tif: order.tif,
            },
        );
        self.match_incoming(ts, seq, sink, order)
    }

    /// Run the incoming order through the book (shared by submissions and
    /// price-change modifies; the latter re-enter as GTC limits and emit no
    /// `OrderAccepted`).
    // One linear pass over the normative matching rules; splitting it would
    // scatter the semantics the property tests are written against.
    #[allow(clippy::too_many_lines)]
    fn match_incoming(
        &mut self,
        ts: SimTime,
        seq: &mut EventSeq,
        sink: &mut dyn EventSink,
        order: NewOrder,
    ) -> Result<(), BookError> {
        let limit = order.kind.limit_price();
        let mut remaining = order.qty;

        // FOK: atomic feasibility pre-check under the STP counting mode that
        // matches what the loop below will actually do.
        if order.tif == TimeInForce::Fok {
            let stp = match self.config.self_match {
                SelfMatchPolicy::Allow => StpCount::All,
                SelfMatchPolicy::CancelResting => StpCount::Skip(order.owner),
                SelfMatchPolicy::CancelAggressor => StpCount::StopAt(order.owner),
            };
            if !self.book.fok_feasible(order.side, limit, remaining, stp) {
                emit(
                    sink,
                    seq,
                    ts,
                    self.symbol,
                    EngineEvent::OrderCancelled {
                        id: order.id,
                        remaining,
                        reason: CancelReason::FokKill,
                    },
                );
                return Ok(());
            }
        }

        let maker_side = order.side.opposite();
        while !remaining.is_zero() {
            let Some(best) = self.book.best_price(maker_side) else {
                break;
            };
            if !crosses(order.side, best, limit) {
                break;
            }
            let front = self
                .book
                .front(maker_side)
                .ok_or_else(|| BookError::Corrupt("best level has no front order".into()))?;

            if front.owner == order.owner {
                match self.config.self_match {
                    SelfMatchPolicy::Allow => {}
                    SelfMatchPolicy::CancelResting => {
                        self.book.remove(front.id)?;
                        emit(
                            sink,
                            seq,
                            ts,
                            self.symbol,
                            EngineEvent::OrderCancelled {
                                id: front.id,
                                remaining: front.remaining,
                                reason: CancelReason::SelfMatch,
                            },
                        );
                        continue;
                    }
                    SelfMatchPolicy::CancelAggressor => {
                        emit(
                            sink,
                            seq,
                            ts,
                            self.symbol,
                            EngineEvent::OrderCancelled {
                                id: order.id,
                                remaining,
                                reason: CancelReason::SelfMatch,
                            },
                        );
                        return Ok(());
                    }
                }
            }

            let fill_qty = remaining.min(front.remaining);
            let maker_remaining = self.book.fill_front(maker_side, fill_qty)?;
            remaining = remaining
                .checked_sub(fill_qty)
                .map_err(|_| BookError::Corrupt("taker remaining underflow".into()))?;
            let trade_id = self.trade_ids.next_trade_id();
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                EngineEvent::Fill {
                    trade_id,
                    maker_id: front.id,
                    taker_id: order.id,
                    maker_owner: front.owner,
                    taker_owner: order.owner,
                    price: front.price,
                    qty: fill_qty,
                    taker_side: order.side,
                    maker_remaining,
                    taker_remaining: remaining,
                },
            );
        }

        if remaining.is_zero() {
            return Ok(());
        }
        match (order.kind, order.tif) {
            (OrderKind::Market, _) => {
                emit(
                    sink,
                    seq,
                    ts,
                    self.symbol,
                    EngineEvent::OrderCancelled {
                        id: order.id,
                        remaining,
                        reason: CancelReason::NoLiquidity,
                    },
                );
            }
            (OrderKind::Limit(price), TimeInForce::Gtc) => {
                self.book.insert(RestingOrder {
                    id: order.id,
                    owner: order.owner,
                    side: order.side,
                    price,
                    remaining,
                })?;
                emit(
                    sink,
                    seq,
                    ts,
                    self.symbol,
                    EngineEvent::OrderRested {
                        id: order.id,
                        price,
                        remaining,
                    },
                );
            }
            (OrderKind::Limit(_), TimeInForce::Ioc) => {
                emit(
                    sink,
                    seq,
                    ts,
                    self.symbol,
                    EngineEvent::OrderCancelled {
                        id: order.id,
                        remaining,
                        reason: CancelReason::IocRemainder,
                    },
                );
            }
            (OrderKind::Limit(_), TimeInForce::Fok) => {
                // Feasibility passed, matching is single-threaded, so a
                // partial FOK is structurally impossible.
                return Err(BookError::Corrupt(
                    "FOK passed feasibility but did not fully fill".into(),
                ));
            }
        }
        Ok(())
    }

    fn modify(
        &mut self,
        ts: SimTime,
        seq: &mut EventSeq,
        sink: &mut dyn EventSink,
        id: sim_core::OrderId,
        new_price: Option<sim_core::Price>,
        new_remaining: Qty,
    ) -> Result<(), BookError> {
        let reject = |reason| EngineEvent::OrderRejected { id, reason };

        let Some(existing) = self.book.get(id) else {
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                reject(RejectReason::UnknownOrder),
            );
            return Ok(());
        };
        if new_remaining.is_zero() {
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                reject(RejectReason::ZeroModifyRemaining),
            );
            return Ok(());
        }
        if let Some(p) = new_price
            && !p.is_positive()
        {
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                reject(RejectReason::NonPositivePrice),
            );
            return Ok(());
        }
        if new_remaining > self.config.max_order_qty {
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                reject(RejectReason::QtyAboveMax),
            );
            return Ok(());
        }

        let target_price = new_price.unwrap_or(existing.price);
        if target_price != existing.price {
            // Cancel+replace under the same id: loses priority, re-enters the
            // full matching path (and may therefore fill immediately).
            self.book.remove(id)?;
            emit(
                sink,
                seq,
                ts,
                self.symbol,
                EngineEvent::OrderModified {
                    id,
                    new_price: target_price,
                    new_remaining,
                    priority_kept: false,
                },
            );
            let reentry = NewOrder {
                id,
                owner: existing.owner,
                symbol: self.symbol,
                side: existing.side,
                kind: OrderKind::Limit(target_price),
                qty: new_remaining,
                tif: TimeInForce::Gtc,
            };
            return self.match_incoming(ts, seq, sink, reentry);
        }

        let priority_kept = match new_remaining.cmp(&existing.remaining) {
            core::cmp::Ordering::Less => {
                self.book.reduce_in_place(id, new_remaining)?;
                true
            }
            core::cmp::Ordering::Greater => {
                self.book.move_to_tail(id, new_remaining)?;
                false
            }
            // No-op modify: acknowledged, position untouched.
            core::cmp::Ordering::Equal => true,
        };
        emit(
            sink,
            seq,
            ts,
            self.symbol,
            EngineEvent::OrderModified {
                id,
                new_price: target_price,
                new_remaining,
                priority_kept,
            },
        );
        Ok(())
    }
}

fn emit(
    sink: &mut dyn EventSink,
    seq: &mut EventSeq,
    ts: SimTime,
    symbol: SymbolId,
    event: EngineEvent,
) {
    sink.emit(EventRecord {
        seq: *seq,
        ts,
        symbol,
        event,
    });
    *seq = seq.next();
}
