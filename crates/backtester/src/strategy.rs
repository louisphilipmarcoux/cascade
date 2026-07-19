//! The `Strategy` trait and its latency-honest adapter onto the simulation.
//!
//! **Structural look-ahead defense**: a strategy's book view is a
//! [`BookProjection`] maintained *solely from market-data records the
//! strategy has received over its delayed feed*. There is no reference to
//! the engine's live book anywhere in this module — a strategy cannot peek
//! at state it hasn't been sent.

use std::collections::BTreeMap;

use market_sim::actor::{Actor, ActorCtx};
use market_sim::projection::{BookProjection, L1};
use sim_core::{
    EngineEvent, EventRecord, OrderId, OrderKind, Price, Qty, Side, SimDuration, SimTime, SymbolId,
    TimeInForce,
};

/// One of our orders, as far as we know from our feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenOrder {
    pub id: OrderId,
    pub symbol: SymbolId,
    pub side: Side,
    /// Limit price once resting (None while market/in flight).
    pub price: Option<Price>,
    pub remaining: Qty,
    /// True once we've seen it rest in the book.
    pub resting: bool,
}

/// One of our fills, as received over the feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnFill {
    pub ts: SimTime,
    pub symbol: SymbolId,
    /// Our side of the trade.
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub is_maker: bool,
    /// Our remaining on that order after the fill.
    pub remaining: Qty,
}

/// Strategy-known own state (positions from received fills, open orders).
#[derive(Debug, Default)]
pub struct OwnState {
    pub positions: BTreeMap<SymbolId, i64>,
    pub open: BTreeMap<OrderId, OpenOrder>,
}

impl OwnState {
    /// Fold one received record; returns the fill if it was ours.
    fn absorb(&mut self, record: &EventRecord) -> Option<OwnFill> {
        match record.event {
            EngineEvent::OrderAccepted { .. } => None,
            EngineEvent::OrderRested {
                id,
                price,
                remaining,
            } => {
                if let Some(order) = self.open.get_mut(&id) {
                    order.price = Some(price);
                    order.remaining = remaining;
                    order.resting = true;
                }
                None
            }
            EngineEvent::OrderCancelled { id, .. } | EngineEvent::OrderRejected { id, .. } => {
                self.open.remove(&id);
                None
            }
            EngineEvent::OrderModified {
                id,
                new_price,
                new_remaining,
                ..
            } => {
                if let Some(order) = self.open.get_mut(&id) {
                    order.price = Some(new_price);
                    order.remaining = new_remaining;
                }
                None
            }
            EngineEvent::Fill {
                maker_id,
                taker_id,
                price,
                qty,
                taker_side,
                maker_remaining,
                taker_remaining,
                ..
            } => {
                // Ours iff we know the order id (we registered it at submit).
                let (our_id, our_side, our_remaining, is_maker) =
                    if self.open.contains_key(&maker_id) {
                        (maker_id, taker_side.opposite(), maker_remaining, true)
                    } else if self.open.contains_key(&taker_id) {
                        (taker_id, taker_side, taker_remaining, false)
                    } else {
                        return None;
                    };
                let lots = i64::try_from(qty.lots()).expect("fill lots fit i64");
                *self.positions.entry(record.symbol).or_insert(0) += match our_side {
                    Side::Buy => lots,
                    Side::Sell => -lots,
                };
                if our_remaining.is_zero() {
                    self.open.remove(&our_id);
                } else if let Some(order) = self.open.get_mut(&our_id) {
                    order.remaining = our_remaining;
                }
                Some(OwnFill {
                    ts: record.ts,
                    symbol: record.symbol,
                    side: our_side,
                    price,
                    qty,
                    is_maker,
                    remaining: our_remaining,
                })
            }
        }
    }
}

/// Everything a strategy can see and do. Read accessors copy small values
/// out (L1, positions) so action methods can borrow mutably without fights.
#[derive(Debug)]
pub struct TradeApi<'a, 'b> {
    now: SimTime,
    books: &'a BTreeMap<SymbolId, BookProjection>,
    own: &'a mut OwnState,
    ctx: &'a mut ActorCtx<'b>,
}

impl TradeApi<'_, '_> {
    #[must_use]
    pub const fn now(&self) -> SimTime {
        self.now
    }

    /// Top of book as we currently know it.
    #[must_use]
    pub fn l1(&self, symbol: SymbolId) -> L1 {
        self.books
            .get(&symbol)
            .map(BookProjection::l1)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn mid_ticks(&self, symbol: SymbolId) -> Option<i64> {
        let l1 = self.l1(symbol);
        match (l1.bid, l1.ask) {
            (Some((b, _)), Some((a, _))) => Some(i64::midpoint(b.ticks(), a.ticks())),
            _ => None,
        }
    }

    #[must_use]
    pub fn position(&self, symbol: SymbolId) -> i64 {
        self.own.positions.get(&symbol).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn open_orders(&self, symbol: SymbolId) -> Vec<OpenOrder> {
        self.own
            .open
            .values()
            .filter(|o| o.symbol == symbol)
            .copied()
            .collect()
    }

    pub fn submit_limit(
        &mut self,
        symbol: SymbolId,
        side: Side,
        price: Price,
        qty: Qty,
    ) -> OrderId {
        self.submit(symbol, side, OrderKind::Limit(price), qty, TimeInForce::Gtc)
    }

    pub fn submit(
        &mut self,
        symbol: SymbolId,
        side: Side,
        kind: OrderKind,
        qty: Qty,
        tif: TimeInForce,
    ) -> OrderId {
        let id = self.ctx.submit(symbol, side, kind, qty, tif);
        self.own.open.insert(
            id,
            OpenOrder {
                id,
                symbol,
                side,
                price: kind.limit_price(),
                remaining: qty,
                resting: false,
            },
        );
        id
    }

    pub fn cancel(&mut self, symbol: SymbolId, id: OrderId) {
        self.ctx.cancel(symbol, id);
    }

    /// Cancel every order we believe is open on `symbol`.
    pub fn cancel_all(&mut self, symbol: SymbolId) {
        let ids: Vec<OrderId> = self
            .own
            .open
            .values()
            .filter(|o| o.symbol == symbol)
            .map(|o| o.id)
            .collect();
        for id in ids {
            self.ctx.cancel(symbol, id);
        }
    }

    pub fn set_timer(&mut self, delay: SimDuration, token: u64) {
        self.ctx.set_timer(delay, token);
    }
}

/// A trading strategy. All knowledge arrives through these callbacks.
pub trait Strategy: std::fmt::Debug + Send {
    fn on_start(&mut self, api: &mut TradeApi<'_, '_>);
    /// Every market-data record received (own-order records included).
    fn on_market_data(&mut self, record: &EventRecord, api: &mut TradeApi<'_, '_>);
    /// Called after state update when a received record was our fill.
    fn on_own_fill(&mut self, fill: &OwnFill, api: &mut TradeApi<'_, '_>);
    fn on_timer(&mut self, token: u64, api: &mut TradeApi<'_, '_>);
    fn on_end(&mut self, api: &mut TradeApi<'_, '_>);
}

/// Adapter: wraps a [`Strategy`] as a market-sim [`Actor`].
#[derive(Debug)]
pub struct StrategyActor {
    strategy: Box<dyn Strategy>,
    books: BTreeMap<SymbolId, BookProjection>,
    own: OwnState,
}

impl StrategyActor {
    #[must_use]
    pub fn new(strategy: Box<dyn Strategy>) -> Self {
        Self {
            strategy,
            books: BTreeMap::new(),
            own: OwnState::default(),
        }
    }

    fn api<'a, 'b>(
        &'a mut self,
        ctx: &'a mut ActorCtx<'b>,
    ) -> (&'a mut Box<dyn Strategy>, TradeApi<'a, 'b>) {
        (
            &mut self.strategy,
            TradeApi {
                now: ctx.now,
                books: &self.books,
                own: &mut self.own,
                ctx,
            },
        )
    }
}

impl Actor for StrategyActor {
    fn on_start(&mut self, ctx: &mut ActorCtx<'_>) {
        let (strategy, mut api) = self.api(ctx);
        strategy.on_start(&mut api);
    }

    fn on_market_data(&mut self, record: &EventRecord, ctx: &mut ActorCtx<'_>) {
        self.books.entry(record.symbol).or_default().apply(record);
        let own_fill = self.own.absorb(record);
        let (strategy, mut api) = self.api(ctx);
        strategy.on_market_data(record, &mut api);
        if let Some(fill) = own_fill {
            strategy.on_own_fill(&fill, &mut api);
        }
    }

    fn on_timer(&mut self, token: u64, ctx: &mut ActorCtx<'_>) {
        let (strategy, mut api) = self.api(ctx);
        strategy.on_timer(token, &mut api);
    }

    fn on_end(&mut self, ctx: &mut ActorCtx<'_>) {
        let (strategy, mut api) = self.api(ctx);
        strategy.on_end(&mut api);
    }
}
