//! Deterministic substrate for quant-sim.
//!
//! Everything the rest of the workspace agrees on lives here: fixed-point
//! market types ([`Price`], [`Qty`], [`Cash`]), identifiers, virtual time
//! ([`SimTime`]), the order/request vocabulary, the engine event stream, and
//! seeded RNG substreams.
//!
//! Design rules enforced across this crate (see `docs/determinism.md`):
//!
//! - **No floats.** All market arithmetic is checked integer arithmetic.
//! - **No I/O, no clock, no threads.** This crate is a pure vocabulary; it
//!   must stay compilable to WASM unchanged.
//! - **Determinism-safe collections only.**

pub mod error;
pub mod events;
pub mod ids;
pub mod instrument;
pub mod ops;
pub mod order;
pub mod price;
pub mod rng;
pub mod time;

#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

pub use error::{ArithmeticError, CoreError};
pub use events::{
    CancelReason, EngineEvent, EventRecord, EventSink, RejectReason, StreamHasher, VecSink,
};
pub use ids::{EventSeq, IdGen, OrderId, OwnerId, SymbolId, TradeId};
pub use instrument::Instrument;
pub use order::{NewOrder, OrderKind, Request, Side, TimeInForce};
pub use price::{Cash, Price, Qty};
pub use rng::SimRng;
pub use time::{SimDuration, SimTime};
