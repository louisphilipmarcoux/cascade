//! External actors (strategies) attached to the simulation.
//!
//! Actors are the *latency-honest* participants: they see market data only
//! through their delayed feed, and their orders travel back through an order
//! link with its own latency. Contrast with background flow sources, which
//! are co-located by construction.
//!
//! Order ids are assigned synchronously at submit time (the actor knows its
//! own ids immediately — think client order ids), while the request itself
//! arrives at the engine after the order-path latency.

use sim_core::{
    EventRecord, OrderId, OrderKind, Qty, Side, SimDuration, SimTime, SymbolId, TimeInForce,
};

/// What an actor can do in response to a callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorAction {
    Submit {
        /// Id pre-assigned by the simulation at submit time.
        id: OrderId,
        symbol: SymbolId,
        side: Side,
        kind: OrderKind,
        qty: Qty,
        tif: TimeInForce,
    },
    Cancel {
        symbol: SymbolId,
        id: OrderId,
    },
    Modify {
        symbol: SymbolId,
        id: OrderId,
        new_price: Option<sim_core::Price>,
        new_remaining: Qty,
    },
    SetTimer {
        delay: SimDuration,
        token: u64,
    },
}

/// Mutable context handed to actor callbacks.
#[derive(Debug)]
pub struct ActorCtx<'a> {
    pub now: SimTime,
    pub(crate) actions: Vec<ActorAction>,
    pub(crate) id_gen: &'a mut sim_core::IdGen,
}

impl ActorCtx<'_> {
    /// Queue a submission; returns the order id the engine will know it by.
    pub fn submit(
        &mut self,
        symbol: SymbolId,
        side: Side,
        kind: OrderKind,
        qty: Qty,
        tif: TimeInForce,
    ) -> OrderId {
        let id = self.id_gen.next_order_id();
        self.actions.push(ActorAction::Submit {
            id,
            symbol,
            side,
            kind,
            qty,
            tif,
        });
        id
    }

    pub fn cancel(&mut self, symbol: SymbolId, id: OrderId) {
        self.actions.push(ActorAction::Cancel { symbol, id });
    }

    pub fn modify(
        &mut self,
        symbol: SymbolId,
        id: OrderId,
        new_price: Option<sim_core::Price>,
        new_remaining: Qty,
    ) {
        self.actions.push(ActorAction::Modify {
            symbol,
            id,
            new_price,
            new_remaining,
        });
    }

    pub fn set_timer(&mut self, delay: SimDuration, token: u64) {
        self.actions.push(ActorAction::SetTimer { delay, token });
    }
}

/// A latency-honest simulation participant.
pub trait Actor: std::fmt::Debug + Send {
    /// Called once at simulation start (virtual t=0).
    fn on_start(&mut self, ctx: &mut ActorCtx<'_>);
    /// A market-data record arrived over this actor's feed.
    fn on_market_data(&mut self, record: &EventRecord, ctx: &mut ActorCtx<'_>);
    /// A previously requested timer fired.
    fn on_timer(&mut self, token: u64, ctx: &mut ActorCtx<'_>);
    /// Called once when the simulation ends.
    fn on_end(&mut self, ctx: &mut ActorCtx<'_>);
}
