//! Price-time-priority limit order book and matching engine.
//!
//! The matching algorithm is written once, generic over the [`book::Book`]
//! trait. The production implementation is [`book::btree::BTreeBook`]
//! (`BTreeMap` price ladder, slab-backed intrusive FIFO levels, O(1) cancel);
//! a bounded `ArrayBook` (M2) implements the same trait for Kani proofs and
//! is differential-tested against it.
//!
//! Normative semantics live in [`engine`] and are enforced three ways: the
//! golden scenario tests (examples), the proptest invariant suite
//! (`tests/properties.rs`, P1–P17), and the Kani harnesses (M2). See this
//! crate's README for the invariant ↔ test ↔ proof table.

pub mod book;
pub mod engine;

#[cfg(any(test, kani, feature = "testkit"))]
pub mod testkit;

#[cfg(kani)]
mod verify;

pub use book::{
    Book, BookError, BookSnapshot, RestingOrder, array::ArrayBook, btree::BTreeBook,
    ladder::LadderBook,
};
pub use engine::{EngineConfig, MatchingEngine, SelfMatchPolicy};
