//! Deterministic discrete-event market simulator.
//!
//! - [`des`]: virtual-clock scheduler (FIFO tie-breaks, P22);
//! - [`source`]: pulled order-flow sources behind one trait — Poisson now,
//!   Hawkes and historical replay at M7, agent populations at Stage 3;
//! - [`latency`]: per-link latency models with portable (`libm`)
//!   transcendentals and fault-injection hooks;
//! - [`sim`]: the orchestrator — engines, sources, links, and the run-global
//!   hashed event log;
//! - [`projection`]: book state rebuilt purely from events (the
//!   event-sourcing consumer side, P13);
//! - [`log`]: integrity-checked binary event-log files.

pub mod actor;
pub mod des;
pub mod latency;
pub mod log;
pub mod projection;
pub mod sim;
pub mod source;

pub use actor::{Actor, ActorAction, ActorCtx};
pub use des::{ActorId, Scheduler, SimEvent, SourceId};
pub use latency::{LatencyModel, LatencySpec};
pub use projection::BookProjection;
pub use sim::{RunLog, SimConfig, Simulation};
pub use source::{BookView, FlowCtx, OrderFlowSource, PlacementModel};
