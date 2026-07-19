//! The order book abstraction.
//!
//! The matching algorithm ([`crate::engine`]) is written once, generic over
//! [`Book`]. Two implementations exist deliberately:
//!
//! - [`btree::BTreeBook`] — the production book (`BTreeMap` price ladder,
//!   slab-backed intrusive FIFO levels, O(1) cancel via an id index);
//! - `ArrayBook` (M2) — a tiny, capacity-bounded book over fixed arrays that
//!   Kani can exhaustively check; differential tests pin it to `BTreeBook`,
//!   which is how proof confidence transfers to the production structure.

pub mod array;
pub mod btree;

use serde::{Deserialize, Serialize};
use sim_core::{OrderId, OwnerId, Price, Qty, Side};

/// An order resting in the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestingOrder {
    pub id: OrderId,
    pub owner: OwnerId,
    pub side: Side,
    pub price: Price,
    pub remaining: Qty,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BookError {
    #[error("order {0} not found")]
    NotFound(OrderId),
    #[error("order {0} already present")]
    Duplicate(OrderId),
    #[error("invalid book operation: {0}")]
    Invalid(String),
    /// Internal structural corruption. Property tests assert this is
    /// unreachable; the engine propagates it instead of panicking.
    #[error("book corrupt: {0}")]
    Corrupt(String),
}

/// How self-match prevention affects the FOK feasibility count.
///
/// - `All`: count every resting order (policy `Allow` — own orders fill).
/// - `Skip`: own orders are excluded (policy `CancelResting` — they will be
///   cancelled and matching continues past them).
/// - `StopAt`: counting stops at the first own order (policy
///   `CancelAggressor` — the aggressor dies there, so deeper liquidity is
///   unreachable). Getting this mode wrong silently breaks FOK atomicity,
///   which is exactly why it is explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StpCount {
    All,
    Skip(OwnerId),
    StopAt(OwnerId),
}

/// Best-first (per side), FIFO-within-level view of the whole book.
/// Test/projection vocabulary — not on any hot path.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub bids: Vec<(Price, Vec<RestingOrder>)>,
    pub asks: Vec<(Price, Vec<RestingOrder>)>,
}

impl BookSnapshot {
    #[must_use]
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.first().map(|(p, _)| *p)
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.first().map(|(p, _)| *p)
    }

    #[must_use]
    pub fn order_count(&self) -> usize {
        self.bids
            .iter()
            .chain(self.asks.iter())
            .map(|(_, orders)| orders.len())
            .sum()
    }
}

/// Order book operations required by the matching algorithm.
///
/// Semantic contract (checked by the shared invariant suite):
/// - levels are FIFO; `insert` enqueues at the tail of its price level;
/// - `fill_front` only ever touches the *front* order of the *best* level;
/// - empty levels never linger;
/// - every id is unique within the book.
pub trait Book {
    /// Enqueue at the tail of the order's price level.
    fn insert(&mut self, order: RestingOrder) -> Result<(), BookError>;

    /// Best price on `side` (highest bid / lowest ask).
    fn best_price(&self, side: Side) -> Option<Price>;

    /// The front (oldest) order of the best level on `side`.
    fn front(&self, side: Side) -> Option<RestingOrder>;

    /// Reduce the front order of the best `side` level by `qty`, removing it
    /// when its remaining reaches zero. Returns the maker's new remaining.
    fn fill_front(&mut self, side: Side, qty: Qty) -> Result<Qty, BookError>;

    /// Remove an order wherever it rests.
    fn remove(&mut self, id: OrderId) -> Result<RestingOrder, BookError>;

    /// Look up a resting order by id.
    fn get(&self, id: OrderId) -> Option<RestingOrder>;

    /// Decrease remaining in place (keeps queue position). `new_remaining`
    /// must be strictly positive and strictly less than the current value.
    fn reduce_in_place(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError>;

    /// Re-queue at the tail of the *same* price level with a new remaining
    /// (the qty-increase modify path).
    fn move_to_tail(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError>;

    /// Would a fill-or-kill for `needed` at `limit` (None = market) fully
    /// execute right now, under the given STP counting mode?
    fn fok_feasible(
        &self,
        taker_side: Side,
        limit: Option<Price>,
        needed: Qty,
        stp: StpCount,
    ) -> bool;

    /// `(total_lots, order_count)` at an exact price level, if present.
    fn level_info(&self, side: Side, price: Price) -> Option<(u128, u32)>;

    /// Total resting orders across both sides.
    fn order_count(&self) -> usize;

    /// Full ordered view (tests, projections, differential comparison).
    fn snapshot(&self) -> BookSnapshot;

    /// Deep structural self-check. Cheap enough to run after every operation
    /// in tests; never called on hot paths.
    fn check_invariants(&self) -> Result<(), BookError>;
}

/// Does a maker price at `maker_price` cross an incoming order at `limit`?
#[must_use]
pub fn crosses(taker_side: Side, maker_price: Price, limit: Option<Price>) -> bool {
    match (taker_side, limit) {
        (_, None) => true,
        (Side::Buy, Some(l)) => maker_price <= l,
        (Side::Sell, Some(l)) => maker_price >= l,
    }
}
