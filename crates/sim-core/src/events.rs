//! The engine event stream — the single source of truth.
//!
//! The matching engine's only output is an append-only, totally ordered
//! sequence of [`EventRecord`]s. Book views, accounting, exports and (later)
//! the WASM demo and distributed replication are all consumers of this
//! stream. L2 book deltas are deliberately *not* engine events; they are a
//! projection rebuilt from order lifecycle events (losslessness is
//! property-tested).

use serde::{Deserialize, Serialize};

use crate::ids::{EventSeq, OrderId, OwnerId, SymbolId, TradeId};
use crate::order::{OrderKind, Side, TimeInForce};
use crate::price::{Price, Qty};
use crate::time::SimTime;

/// Why a submission (or an operation on an existing order) was rejected.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    ZeroQty,
    NonPositivePrice,
    DuplicateOrderId,
    MarketGtc,
    QtyAboveMax,
    UnknownOrder,
    ZeroModifyRemaining,
}

/// Why a (remainder of an) order left the book without (fully) trading.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    /// Explicit user cancel.
    User,
    /// IOC remainder after matching.
    IocRemainder,
    /// FOK feasibility check failed; nothing traded.
    FokKill,
    /// Market order exhausted the opposite book.
    NoLiquidity,
    /// Removed by self-match prevention.
    SelfMatch,
}

/// A single engine event. Every variant is a primitive lifecycle fact.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    /// Acknowledgement: the submission passed validation. Always the first
    /// event of a valid submit, carrying the original quantity.
    OrderAccepted {
        id: OrderId,
        owner: OwnerId,
        side: Side,
        kind: OrderKind,
        qty: Qty,
        tif: TimeInForce,
    },
    OrderRejected {
        id: OrderId,
        reason: RejectReason,
    },
    /// One maker/taker match. `price` is always the maker's resting price.
    Fill {
        trade_id: TradeId,
        maker_id: OrderId,
        taker_id: OrderId,
        maker_owner: OwnerId,
        taker_owner: OwnerId,
        price: Price,
        qty: Qty,
        taker_side: Side,
        maker_remaining: Qty,
        taker_remaining: Qty,
    },
    /// A GTC remainder joined the book.
    OrderRested {
        id: OrderId,
        price: Price,
        remaining: Qty,
    },
    /// `remaining` is the quantity that left the book un-traded.
    OrderCancelled {
        id: OrderId,
        remaining: Qty,
        reason: CancelReason,
    },
    OrderModified {
        id: OrderId,
        new_price: Price,
        new_remaining: Qty,
        priority_kept: bool,
    },
}

/// An event with its total-order metadata.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
pub struct EventRecord {
    pub seq: EventSeq,
    pub ts: SimTime,
    pub symbol: SymbolId,
    pub event: EngineEvent,
}

/// Consumer of the event stream.
pub trait EventSink {
    fn emit(&mut self, record: EventRecord);
}

/// Collects events into memory (tests, small runs).
#[derive(Debug, Default, Clone)]
pub struct VecSink {
    pub records: Vec<EventRecord>,
}

impl VecSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventSink for VecSink {
    fn emit(&mut self, record: EventRecord) {
        self.records.push(record);
    }
}

/// Incrementally hashes the serialized event stream (BLAKE3 over bincode
/// records). Two runs are byte-identical iff their hashes match; every run
/// report carries this hash and `quant-sim run --verify-hash` re-checks it.
pub struct StreamHasher {
    hasher: blake3::Hasher,
    count: u64,
}

impl core::fmt::Debug for StreamHasher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StreamHasher")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl StreamHasher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
            count: 0,
        }
    }

    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Hex digest of everything emitted so far.
    #[must_use]
    pub fn finalize_hex(&self) -> String {
        self.hasher.finalize().to_hex().to_string()
    }
}

impl Default for StreamHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for StreamHasher {
    fn emit(&mut self, record: EventRecord) {
        // Native bincode encoding: stable little-endian layout, identical on
        // every platform, independent of the serde (JSON) representation.
        let bytes = bincode::encode_to_vec(record, bincode::config::standard())
            .expect("EventRecord encoding is infallible");
        self.hasher.update(&bytes);
        self.count += 1;
    }
}

/// Fans one stream out to two sinks (e.g. collect + hash).
#[derive(Debug)]
pub struct TeeSink<'a, A: EventSink, B: EventSink> {
    pub first: &'a mut A,
    pub second: &'a mut B,
}

impl<A: EventSink, B: EventSink> EventSink for TeeSink<'_, A, B> {
    fn emit(&mut self, record: EventRecord) {
        self.first.emit(record);
        self.second.emit(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> EventRecord {
        EventRecord {
            seq: EventSeq::new(7),
            ts: SimTime::from_millis(1),
            symbol: SymbolId::new(0),
            event: EngineEvent::Fill {
                trade_id: TradeId::new(1),
                maker_id: OrderId::new(10),
                taker_id: OrderId::new(11),
                maker_owner: OwnerId::new(1),
                taker_owner: OwnerId::new(2),
                price: Price::new(100),
                qty: Qty::new(5),
                taker_side: Side::Buy,
                maker_remaining: Qty::new(0),
                taker_remaining: Qty::new(2),
            },
        }
    }

    #[test]
    fn stream_hash_is_deterministic_and_order_sensitive() {
        let r1 = sample_record();
        let mut r2 = sample_record();
        r2.seq = EventSeq::new(8);

        let mut a = StreamHasher::new();
        a.emit(r1);
        a.emit(r2);
        let mut b = StreamHasher::new();
        b.emit(r1);
        b.emit(r2);
        assert_eq!(a.finalize_hex(), b.finalize_hex());
        assert_eq!(a.count(), 2);

        let mut c = StreamHasher::new();
        c.emit(r2);
        c.emit(r1);
        assert_ne!(a.finalize_hex(), c.finalize_hex());
    }

    #[test]
    fn json_round_trip_preserves_record() {
        let r = sample_record();
        let json = serde_json::to_string(&r).unwrap();
        let back: EventRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
        // Tagged representation is part of the export contract.
        assert!(json.contains("\"type\":\"fill\""));
    }

    #[test]
    fn bincode_round_trip_preserves_record() {
        let r = sample_record();
        let bytes = bincode::encode_to_vec(r, bincode::config::standard()).unwrap();
        let (back, _): (EventRecord, usize) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(r, back);
    }
}
