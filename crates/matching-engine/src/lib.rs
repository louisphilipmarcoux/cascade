//! Price-time-priority limit order book and matching engine.
//!
//! Lands at M1: `Book` trait, `BTreeMap` book (slab + intrusive FIFO levels),
//! matching semantics (STP, IOC/FOK, modify), the proptest invariant suite
//! (P1–P18), and at M2 the Kani-provable `ArrayBook` + proof harnesses.
