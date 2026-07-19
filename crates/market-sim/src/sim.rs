//! The simulation orchestrator: engines, flow sources, latency links, and
//! the run-global event log with its BLAKE3 hash.
//!
//! A [`Simulation`] is a pure function of `(configuration, seed)`. Sources
//! draw from their own RNG substreams, all latency lives on links, and every
//! emitted engine record lands in one totally ordered, hashed log.

use std::collections::BTreeMap;

use matching_engine::{BTreeBook, EngineConfig, MatchingEngine};
use sim_core::rng::{Rng, SimRng, StreamId};
use sim_core::{
    EventRecord, EventSeq, EventSink, IdGen, OwnerId, Request, SimTime, StreamHasher, SymbolId,
};

use crate::actor::{Actor, ActorAction, ActorCtx};
use crate::des::{ActorId, Scheduler, SimEvent, SourceId};
use crate::latency::{LatencyModel, LatencySpec};
use crate::source::{FlowCtx, OrderFlowSource};

/// The run-global event log: collected records + incremental hash.
#[derive(Debug, Default)]
pub struct RunLog {
    pub records: Vec<EventRecord>,
    hasher: StreamHasher,
}

impl RunLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Hex BLAKE3 of everything logged so far.
    #[must_use]
    pub fn hash_hex(&self) -> String {
        self.hasher.finalize_hex()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl EventSink for RunLog {
    fn emit(&mut self, record: EventRecord) {
        self.hasher.emit(record);
        self.records.push(record);
    }
}

struct SourceSlot {
    id: SourceId,
    symbol: SymbolId,
    owner: OwnerId,
    source: Box<dyn OrderFlowSource>,
    rng: Rng,
    order_latency: Box<dyn LatencyModel>,
    exhausted: bool,
}

struct ActorSlot {
    id: ActorId,
    #[allow(dead_code)] // read by the backtester via `actor_owner`
    owner: OwnerId,
    subscriptions: std::collections::BTreeSet<SymbolId>,
    actor: Box<dyn Actor>,
    rng: Rng,
    order_latency: Box<dyn LatencyModel>,
    md_latency: Box<dyn LatencyModel>,
}

impl std::fmt::Debug for ActorSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorSlot")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SourceSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceSlot")
            .field("id", &self.id)
            .field("symbol", &self.symbol)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Run bounds.
#[derive(Debug, Clone, Copy)]
pub struct SimConfig {
    pub seed: u64,
    pub t_end: SimTime,
}

/// The deterministic market simulation.
#[derive(Debug)]
pub struct Simulation {
    config: SimConfig,
    scheduler: Scheduler,
    engines: BTreeMap<SymbolId, MatchingEngine<BTreeBook>>,
    sources: Vec<SourceSlot>,
    actors: Vec<ActorSlot>,
    /// owner → source index, for zero-latency own-order feedback.
    owner_to_source: BTreeMap<OwnerId, usize>,
    /// Live order → owner, so ownerless records (rest/cancel/modify) can be
    /// routed to the source that created the order. Pruned on terminal events.
    id_owner: BTreeMap<sim_core::OrderId, OwnerId>,
    id_gen: IdGen,
    seq: EventSeq,
    pub log: RunLog,
    rng_root: SimRng,
}

impl Simulation {
    #[must_use]
    pub fn new(config: SimConfig) -> Self {
        Self {
            config,
            scheduler: Scheduler::new(),
            engines: BTreeMap::new(),
            sources: Vec::new(),
            actors: Vec::new(),
            owner_to_source: BTreeMap::new(),
            id_owner: BTreeMap::new(),
            id_gen: IdGen::new(),
            seq: EventSeq::new(0),
            log: RunLog::new(),
            rng_root: SimRng::new(config.seed),
        }
    }

    pub fn add_engine(&mut self, symbol: SymbolId, engine_config: EngineConfig) {
        self.engines
            .insert(symbol, MatchingEngine::new_btree(symbol, engine_config));
    }

    /// Register a flow source for `symbol`; each source gets its own RNG
    /// substream (derived from its index) and its own order-path latency.
    pub fn add_source(
        &mut self,
        symbol: SymbolId,
        owner: OwnerId,
        source: Box<dyn OrderFlowSource>,
        order_latency: &LatencySpec,
    ) {
        let index = self.sources.len();
        let id = SourceId(u32::try_from(index).expect("source count fits u32"));
        let rng = self
            .rng_root
            .stream(StreamId(StreamId::DYNAMIC_BASE + index as u64));
        self.owner_to_source.insert(owner, index);
        self.sources.push(SourceSlot {
            id,
            symbol,
            owner,
            source,
            rng,
            order_latency: order_latency.build(),
            exhausted: false,
        });
    }

    /// Register a latency-honest actor (strategy). Returns its id; the
    /// `owner` identifies its orders in the event stream.
    pub fn add_actor(
        &mut self,
        owner: OwnerId,
        subscriptions: impl IntoIterator<Item = SymbolId>,
        actor: Box<dyn Actor>,
        order_latency: &LatencySpec,
        md_latency: &LatencySpec,
    ) -> ActorId {
        let index = self.actors.len();
        let id = ActorId(u32::try_from(index).expect("actor count fits u32"));
        let rng = self
            .rng_root
            .stream(StreamId(StreamId::DYNAMIC_BASE + 100_000 + index as u64));
        self.actors.push(ActorSlot {
            id,
            owner,
            subscriptions: subscriptions.into_iter().collect(),
            actor,
            rng,
            order_latency: order_latency.build(),
            md_latency: md_latency.build(),
        });
        id
    }

    #[must_use]
    pub fn actor_owner(&self, id: ActorId) -> Option<OwnerId> {
        self.actors.get(id.0 as usize).map(|slot| slot.owner)
    }

    #[must_use]
    pub fn engine(&self, symbol: SymbolId) -> Option<&MatchingEngine<BTreeBook>> {
        self.engines.get(&symbol)
    }

    #[must_use]
    pub const fn now(&self) -> SimTime {
        self.scheduler.now()
    }

    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.config.seed
    }

    /// Ask source `index` for its next action and schedule it.
    fn wake_source(&mut self, index: usize) {
        let slot = &mut self.sources[index];
        if slot.exhausted {
            return;
        }
        let engine = self
            .engines
            .get(&slot.symbol)
            .expect("source registered for unknown symbol");
        let mut ctx = FlowCtx {
            now: self.scheduler.now(),
            book: engine.book(),
            id_gen: &mut self.id_gen,
            rng: &mut slot.rng,
        };
        match slot.source.next(&mut ctx) {
            Some((at, request)) => {
                let delay = slot.order_latency.sample(&mut slot.rng);
                let deliver_at = at.checked_add(delay).unwrap_or(at);
                self.scheduler.schedule(
                    deliver_at,
                    SimEvent::EngineRequest {
                        symbol: slot.symbol,
                        request,
                    },
                );
                // Wake again when this action fires, to produce the next one.
                let source = slot.id;
                self.scheduler.schedule(at, SimEvent::SourceWake { source });
            }
            None => slot.exhausted = true,
        }
    }

    fn process_engine_request(&mut self, symbol: SymbolId, request: Request) {
        use sim_core::EngineEvent as E;
        let engine = self
            .engines
            .get_mut(&symbol)
            .expect("request for unknown symbol");
        let start = self.log.records.len();
        engine
            .process(self.scheduler.now(), &mut self.seq, &mut self.log, request)
            .expect("engine corruption");
        // Fan records out to subscribed actors over their (latency-sampled)
        // market-data links.
        for i in start..self.log.records.len() {
            let record = self.log.records[i];
            let now = self.scheduler.now();
            for actor_index in 0..self.actors.len() {
                if !self.actors[actor_index]
                    .subscriptions
                    .contains(&record.symbol)
                {
                    continue;
                }
                let slot = &mut self.actors[actor_index];
                let delay = slot.md_latency.sample(&mut slot.rng);
                let at = now.checked_add(delay).unwrap_or(now);
                self.scheduler.schedule(
                    at,
                    SimEvent::MarketData {
                        subscriber: slot.id,
                        record,
                    },
                );
            }
        }
        // Zero-latency feedback of own-order records to background sources,
        // routed via the id→owner registry (rest/cancel/modify events carry
        // no owner field).
        for i in start..self.log.records.len() {
            let record = self.log.records[i];
            match record.event {
                E::OrderAccepted { id, owner, .. } => {
                    self.id_owner.insert(id, owner);
                    self.route(owner, &record);
                }
                E::Fill {
                    maker_id,
                    taker_id,
                    maker_remaining,
                    taker_remaining,
                    ..
                } => {
                    for (order, remaining) in
                        [(maker_id, maker_remaining), (taker_id, taker_remaining)]
                    {
                        if let Some(&owner) = self.id_owner.get(&order) {
                            self.route(owner, &record);
                            if remaining.is_zero() {
                                self.id_owner.remove(&order);
                            }
                        }
                    }
                }
                E::OrderCancelled { id, .. } => {
                    if let Some(owner) = self.id_owner.remove(&id) {
                        self.route(owner, &record);
                    }
                }
                E::OrderRested { id, .. }
                | E::OrderModified { id, .. }
                | E::OrderRejected { id, .. } => {
                    if let Some(&owner) = self.id_owner.get(&id) {
                        self.route(owner, &record);
                    }
                }
            }
        }
    }

    fn route(&mut self, owner: OwnerId, record: &EventRecord) {
        if let Some(&source_index) = self.owner_to_source.get(&owner) {
            self.sources[source_index].source.on_own_record(record);
        }
    }

    /// Deliver one actor callback and apply the actions it queued.
    fn with_actor(
        &mut self,
        index: usize,
        call: impl FnOnce(&mut Box<dyn Actor>, &mut ActorCtx<'_>),
    ) {
        let mut ctx = ActorCtx {
            now: self.scheduler.now(),
            actions: Vec::new(),
            id_gen: &mut self.id_gen,
        };
        call(&mut self.actors[index].actor, &mut ctx);
        let actions = ctx.actions;
        let now = self.scheduler.now();
        for action in actions {
            match action {
                ActorAction::Submit {
                    id,
                    symbol,
                    side,
                    kind,
                    qty,
                    tif,
                } => {
                    let slot = &mut self.actors[index];
                    let delay = slot.order_latency.sample(&mut slot.rng);
                    let at = now.checked_add(delay).unwrap_or(now);
                    let request = Request::Submit(sim_core::NewOrder {
                        id,
                        owner: slot.owner,
                        symbol,
                        side,
                        kind,
                        qty,
                        tif,
                    });
                    self.scheduler
                        .schedule(at, SimEvent::EngineRequest { symbol, request });
                }
                ActorAction::Cancel { symbol, id } => {
                    let slot = &mut self.actors[index];
                    let delay = slot.order_latency.sample(&mut slot.rng);
                    let at = now.checked_add(delay).unwrap_or(now);
                    self.scheduler.schedule(
                        at,
                        SimEvent::EngineRequest {
                            symbol,
                            request: Request::Cancel { id },
                        },
                    );
                }
                ActorAction::Modify {
                    symbol,
                    id,
                    new_price,
                    new_remaining,
                } => {
                    let slot = &mut self.actors[index];
                    let delay = slot.order_latency.sample(&mut slot.rng);
                    let at = now.checked_add(delay).unwrap_or(now);
                    self.scheduler.schedule(
                        at,
                        SimEvent::EngineRequest {
                            symbol,
                            request: Request::Modify {
                                id,
                                new_price,
                                new_remaining,
                            },
                        },
                    );
                }
                ActorAction::SetTimer { delay, token } => {
                    let at = now.checked_add(delay).unwrap_or(now);
                    let owner = self.actors[index].id;
                    self.scheduler
                        .schedule(at, SimEvent::Timer { owner, token });
                }
            }
        }
    }

    /// Run to `t_end` (or queue exhaustion). Returns the final event count.
    pub fn run(&mut self) -> usize {
        for index in 0..self.sources.len() {
            self.wake_source(index);
        }
        for index in 0..self.actors.len() {
            self.with_actor(index, |actor, ctx| actor.on_start(ctx));
        }
        self.scheduler
            .schedule(self.config.t_end, SimEvent::EndOfSim);

        while let Some((_at, event)) = self.scheduler.pop() {
            match event {
                SimEvent::EndOfSim => break,
                SimEvent::SourceWake { source } => self.wake_source(source.0 as usize),
                SimEvent::EngineRequest { symbol, request } => {
                    self.process_engine_request(symbol, request);
                }
                SimEvent::MarketData { subscriber, record } => {
                    self.with_actor(subscriber.0 as usize, |actor, ctx| {
                        actor.on_market_data(&record, ctx);
                    });
                }
                SimEvent::Timer { owner, token } => {
                    self.with_actor(owner.0 as usize, |actor, ctx| actor.on_timer(token, ctx));
                }
                SimEvent::SampleTick => {}
            }
        }
        for index in 0..self.actors.len() {
            self.with_actor(index, |actor, ctx| actor.on_end(ctx));
        }
        self.log.len()
    }
}
