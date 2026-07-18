//! Order and request vocabulary.

use serde::{Deserialize, Serialize};

use crate::ids::{OrderId, OwnerId, SymbolId};
use crate::price::{Price, Qty};

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
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    /// +1 for buy, −1 for sell.
    #[must_use]
    pub const fn sign(self) -> i64 {
        match self {
            Self::Buy => 1,
            Self::Sell => -1,
        }
    }
}

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
pub enum OrderKind {
    Limit(Price),
    Market,
}

impl OrderKind {
    #[must_use]
    pub const fn limit_price(self) -> Option<Price> {
        match self {
            Self::Limit(p) => Some(p),
            Self::Market => None,
        }
    }
}

/// Time in force. `Market + Gtc` is rejected at validation (market orders
/// never rest).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    bincode::Encode,
    bincode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    #[default]
    Gtc,
    Ioc,
    Fok,
}

/// A fully-specified order submission.
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
pub struct NewOrder {
    pub id: OrderId,
    pub owner: OwnerId,
    pub symbol: SymbolId,
    pub side: Side,
    pub kind: OrderKind,
    pub qty: Qty,
    pub tif: TimeInForce,
}

/// A request into the matching engine.
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
pub enum Request {
    Submit(NewOrder),
    Cancel {
        id: OrderId,
    },
    /// Modify semantics (normative, see matching-engine README):
    /// - `new_price` change ⇒ cancel+replace under the same id; re-enters
    ///   matching; loses time priority.
    /// - same price, `new_remaining` decrease ⇒ in place, keeps priority.
    /// - same price, `new_remaining` increase ⇒ moves to tail of its level.
    /// - `new_remaining == 0` ⇒ rejected (use `Cancel`).
    Modify {
        id: OrderId,
        new_price: Option<Price>,
        new_remaining: Qty,
    },
}

impl Request {
    /// The order id this request concerns.
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        match self {
            Self::Submit(o) => o.id,
            Self::Cancel { id } | Self::Modify { id, .. } => *id,
        }
    }
}
