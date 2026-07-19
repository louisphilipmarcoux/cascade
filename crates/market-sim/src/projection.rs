//! Book projections rebuilt purely from the event stream.
//!
//! This is the event-sourcing consumer side: given only `EventRecord`s, the
//! projection maintains aggregate L1/L2 state. Its equivalence with the
//! engine's own book is a tested invariant (P13) — which is what licenses
//! every downstream consumer (exports, strategies' delayed views, later the
//! WASM demo) to trust the stream alone.

use std::collections::BTreeMap;

use sim_core::{EngineEvent, EventRecord, OrderId, Price, Qty, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    side: Side,
    price: Price,
    remaining: Qty,
}

/// Aggregate ladder projection (qty and order count per price level).
#[derive(Debug, Default, Clone)]
pub struct BookProjection {
    bids: BTreeMap<Price, (u128, u32)>,
    asks: BTreeMap<Price, (u128, u32)>,
    orders: BTreeMap<OrderId, Placed>,
    pending_sides: PendingSides,
}

/// One L1 (top-of-book) observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct L1 {
    pub bid: Option<(Price, u128)>,
    pub ask: Option<(Price, u128)>,
}

impl BookProjection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ladder(&mut self, side: Side) -> &mut BTreeMap<Price, (u128, u32)> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    fn add(&mut self, id: OrderId, side: Side, price: Price, remaining: Qty) {
        let entry = self.ladder(side).entry(price).or_insert((0, 0));
        entry.0 += u128::from(remaining.lots());
        entry.1 += 1;
        self.orders.insert(
            id,
            Placed {
                side,
                price,
                remaining,
            },
        );
    }

    fn take(&mut self, id: OrderId) -> Option<Placed> {
        let placed = self.orders.remove(&id)?;
        let ladder = self.ladder(placed.side);
        if let Some(entry) = ladder.get_mut(&placed.price) {
            entry.0 -= u128::from(placed.remaining.lots());
            entry.1 -= 1;
            if entry.1 == 0 {
                ladder.remove(&placed.price);
            }
        }
        Some(placed)
    }

    /// Fold one record into the projection.
    pub fn apply(&mut self, record: &EventRecord) {
        match record.event {
            EngineEvent::OrderRested {
                id,
                price,
                remaining,
            } => {
                // Determine side from a previous placement (modify re-entry)
                // is impossible here: Rested always follows an Accepted or a
                // Modified that removed the order; side arrives via Accepted.
                // The projection therefore tracks sides from Accepted events.
                if let Some(side) = self.pending_sides.remove(id) {
                    self.add(id, side, price, remaining);
                }
            }
            EngineEvent::OrderAccepted { id, side, .. } => {
                self.pending_sides.insert(id, side);
            }
            EngineEvent::Fill {
                maker_id,
                maker_remaining,
                ..
            } => {
                // `take` must run unconditionally (removes the maker); the
                // let-chain evaluates it before the remaining check.
                if let Some(placed) = self.take(maker_id)
                    && !maker_remaining.is_zero()
                {
                    self.add(maker_id, placed.side, placed.price, maker_remaining);
                    self.pending_sides.remove(maker_id);
                }
            }
            EngineEvent::OrderCancelled { id, .. } => {
                self.take(id);
            }
            EngineEvent::OrderModified {
                id,
                new_price,
                new_remaining,
                ..
            } => {
                if let Some(placed) = self.take(id) {
                    if new_price == placed.price {
                        self.add(id, placed.side, placed.price, new_remaining);
                    } else {
                        // Price change: order is in flight; a following
                        // Fill/Rested will place it. Remember its side.
                        self.pending_sides.insert(id, placed.side);
                    }
                }
            }
            EngineEvent::OrderRejected { .. } => {}
        }
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<(Price, u128)> {
        self.bids.iter().next_back().map(|(p, (q, _))| (*p, *q))
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<(Price, u128)> {
        self.asks.iter().next().map(|(p, (q, _))| (*p, *q))
    }

    #[must_use]
    pub fn l1(&self) -> L1 {
        L1 {
            bid: self.best_bid(),
            ask: self.best_ask(),
        }
    }

    /// `(total_lots, order_count)` at a level.
    #[must_use]
    pub fn level(&self, side: Side, price: Price) -> Option<(u128, u32)> {
        match side {
            Side::Buy => self.bids.get(&price).copied(),
            Side::Sell => self.asks.get(&price).copied(),
        }
    }

    #[must_use]
    pub fn resting_orders(&self) -> usize {
        self.orders.len()
    }
}

// Sides of accepted-but-not-yet-rested orders (and in-flight modifies).
#[derive(Debug, Default, Clone)]
struct PendingSides(BTreeMap<OrderId, Side>);

impl PendingSides {
    fn insert(&mut self, id: OrderId, side: Side) {
        self.0.insert(id, side);
    }
    fn remove(&mut self, id: OrderId) -> Option<Side> {
        self.0.remove(&id)
    }
}
