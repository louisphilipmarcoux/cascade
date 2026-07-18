//! Identifier newtypes.
//!
//! All ids are plain integers with no embedded meaning. [`OrderId`]s are
//! assigned by a single monotonic [`IdGen`] owned by the simulation (the
//! engine validates uniqueness and rejects duplicates); [`TradeId`]s are
//! assigned by the engine; [`EventSeq`] totally orders the event stream.

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($(#[$doc:meta])* $name:ident, $inner:ty) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
            Default, bincode::Encode, bincode::Decode,
        )]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl $name {
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn value(self) -> $inner {
                self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_newtype!(
    /// Order identifier, unique within a run.
    OrderId,
    u64
);
id_newtype!(
    /// Trade identifier, assigned by the engine per fill.
    TradeId,
    u64
);
id_newtype!(
    /// Instrument identifier (dense, assigned at scenario load).
    SymbolId,
    u16
);
id_newtype!(
    /// Participant identifier: which strategy/agent/flow-source owns an order.
    /// Required for self-match prevention and fill routing.
    OwnerId,
    u16
);
id_newtype!(
    /// Strictly monotonic sequence number totally ordering the event stream.
    EventSeq,
    u64
);

impl EventSeq {
    /// The next sequence number.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Monotonic id generator. One instance per id space per run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdGen {
    next: u64,
}

impl IdGen {
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 0 }
    }

    /// Starts issuing at `first` (useful for reserving low ids).
    #[must_use]
    pub const fn starting_at(first: u64) -> Self {
        Self { next: first }
    }

    pub const fn next_u64(&mut self) -> u64 {
        let v = self.next;
        self.next += 1;
        v
    }

    pub const fn next_order_id(&mut self) -> OrderId {
        OrderId(self.next_u64())
    }

    pub const fn next_trade_id(&mut self) -> TradeId {
        TradeId(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idgen_is_monotonic_from_zero() {
        let mut g = IdGen::new();
        assert_eq!(g.next_order_id(), OrderId(0));
        assert_eq!(g.next_order_id(), OrderId(1));
        assert_eq!(g.next_u64(), 2);
    }

    #[test]
    fn event_seq_next_increments() {
        assert_eq!(EventSeq(41).next(), EventSeq(42));
    }
}
