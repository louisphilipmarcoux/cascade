//! Order-flow sources: where background market activity comes from.
//!
//! A source is pulled by the simulation: at each wake it returns the absolute
//! time of its next action and the request to perform. Timing (the point
//! process) is deliberately decoupled from *placement* (where limit orders
//! land and how big they are) so that Poisson, Hawkes and agent-based flows
//! differ only in the part that matters.

pub mod hawkes;
pub mod poisson;
pub mod replay;

use rand::Rng as _;
use serde::{Deserialize, Serialize};
use sim_core::rng::Rng;
use sim_core::{IdGen, NewOrder, OrderId, Price, Qty, Request, SimTime};

use crate::latency::exponential_ns;

/// Read-only book view offered to sources (and later strategies).
pub trait BookView {
    fn best_bid(&self) -> Option<Price>;
    fn best_ask(&self) -> Option<Price>;
}

impl<B: matching_engine::Book> BookView for B {
    fn best_bid(&self) -> Option<Price> {
        self.best_price(sim_core::Side::Buy)
    }
    fn best_ask(&self) -> Option<Price> {
        self.best_price(sim_core::Side::Sell)
    }
}

/// Context handed to a source when it is asked for its next action.
pub struct FlowCtx<'a> {
    pub now: SimTime,
    pub book: &'a dyn BookView,
    pub id_gen: &'a mut IdGen,
    pub rng: &'a mut Rng,
}

impl std::fmt::Debug for FlowCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// A pulled order-flow source.
pub trait OrderFlowSource: std::fmt::Debug + Send {
    /// The next action and its absolute time (≥ `ctx.now`), or `None` when
    /// the source is exhausted.
    fn next(&mut self, ctx: &mut FlowCtx<'_>) -> Option<(SimTime, Request)>;

    /// Feedback: an engine record concerning one of this source's own orders
    /// (zero-latency co-located feedback for background flow — documented).
    fn on_own_record(&mut self, record: &sim_core::EventRecord);
}

/// Where limit orders land relative to the touch, and how big they are.
///
/// Depth from the reference price is geometric (each additional tick with
/// probability `deeper_p`), sizes are log-normal in lots. This is the
/// classic zero-intelligence placement shape; its parameters are scenario
/// configuration, and its realism is *measured* by the stylized-facts
/// battery rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PlacementModel {
    /// P(place one tick deeper), geometric depth from the touch.
    pub deeper_p: f64,
    /// Hard cap on depth in ticks.
    pub max_depth_ticks: i64,
    /// Log-normal size: mean of log(lots).
    pub size_log_mu: f64,
    /// Log-normal size: std dev of log(lots).
    pub size_log_sigma: f64,
    /// Hard cap on size in lots.
    pub max_lots: u64,
}

impl Default for PlacementModel {
    fn default() -> Self {
        Self {
            deeper_p: 0.65,
            max_depth_ticks: 50,
            size_log_mu: 1.2,
            size_log_sigma: 0.9,
            max_lots: 500,
        }
    }
}

impl PlacementModel {
    /// Depth in ticks from the reference price (≥ 1).
    pub fn depth_ticks(&self, rng: &mut Rng) -> i64 {
        let mut depth: i64 = 1;
        while depth < self.max_depth_ticks && rng.random::<f64>() < self.deeper_p {
            depth += 1;
        }
        depth
    }

    /// Order size in lots (≥ 1).
    pub fn size_lots(&self, rng: &mut Rng) -> u64 {
        let z = crate::latency::standard_normal(rng);
        let lots = libm::exp(self.size_log_mu + self.size_log_sigma * z);
        (lots as u64).clamp(1, self.max_lots)
    }
}

/// Exponential inter-arrival helper shared by Poisson-timed processes.
pub(crate) fn next_arrival(now: SimTime, rate_per_sec: f64, rng: &mut Rng) -> SimTime {
    let mean_ns = 1e9 / rate_per_sec;
    let dt = exponential_ns(rng, mean_ns).min(i64::MAX as f64) as i64;
    now.checked_add(sim_core::SimDuration::from_nanos(dt.max(1)))
        .unwrap_or(SimTime::from_nanos(u64::MAX))
}

/// Bookkeeping helper: track which of a source's orders still rest.
#[derive(Debug, Default)]
pub struct OwnOrders {
    resting: std::collections::BTreeSet<OrderId>,
}

impl OwnOrders {
    pub fn on_record(&mut self, record: &sim_core::EventRecord) {
        use sim_core::EngineEvent as E;
        match record.event {
            E::OrderRested { id, .. } => {
                self.resting.insert(id);
            }
            E::OrderCancelled { id, .. } => {
                self.resting.remove(&id);
            }
            E::Fill {
                maker_id,
                maker_remaining,
                ..
            } => {
                if maker_remaining.is_zero() {
                    self.resting.remove(&maker_id);
                }
            }
            E::OrderModified { .. } | E::OrderAccepted { .. } | E::OrderRejected { .. } => {}
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.resting.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.resting.is_empty()
    }

    /// Pick the `idx % len`-th resting order (deterministic; `BTreeSet`
    /// iteration is ordered).
    #[must_use]
    pub fn pick(&self, idx: usize) -> Option<OrderId> {
        if self.resting.is_empty() {
            None
        } else {
            self.resting.iter().nth(idx % self.resting.len()).copied()
        }
    }

    /// All resting ids, in deterministic (ordered) sequence.
    #[must_use]
    pub fn ids(&self) -> Vec<OrderId> {
        self.resting.iter().copied().collect()
    }
}

/// Convenience constructor for source-submitted orders.
pub(crate) fn submit(
    id_gen: &mut IdGen,
    owner: sim_core::OwnerId,
    symbol: sim_core::SymbolId,
    side: sim_core::Side,
    kind: sim_core::OrderKind,
    qty: Qty,
    tif: sim_core::TimeInForce,
) -> (OrderId, Request) {
    let id = id_gen.next_order_id();
    (
        id,
        Request::Submit(NewOrder {
            id,
            owner,
            symbol,
            side,
            kind,
            qty,
            tif,
        }),
    )
}
