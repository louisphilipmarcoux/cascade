//! The verification operation model.
//!
//! [`OpTemplate`] is the *shared vocabulary* between the proptest suites and
//! the Kani proof harnesses in `matching-engine`: both drive the engine
//! through sequences of these templates, and both check the same invariant
//! predicates afterwards. Keeping the model here (plain data, no test-crate
//! dependencies) is what makes "the property tests and the proofs talk about
//! the same semantics" a checkable statement rather than a claim.
//!
//! Templates are *relative*: submissions carry no order id (the harness
//! assigns fresh monotonic ids), and cancels/modifies address previously
//! issued orders by index. This keeps generated sequences meaningful — a
//! random absolute id would almost never hit a live order.

use crate::order::{OrderKind, Side, TimeInForce};
use crate::price::{Price, Qty};

/// Bounds for generated order domains.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    pub max_price_ticks: i64,
    pub max_qty_lots: u64,
    pub owners: u16,
}

impl Domain {
    /// Default proptest domain: small enough that prices collide constantly
    /// (which is what exercises matching), large enough to be interesting.
    pub const PROPTEST: Self = Self {
        max_price_ticks: 40,
        max_qty_lots: 20,
        owners: 3,
    };

    /// Kani-scale domain (tiny, symbolic-friendly).
    pub const KANI: Self = Self {
        max_price_ticks: 4,
        max_qty_lots: 3,
        owners: 2,
    };
}

/// Parameters of a submission (id assigned by the harness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitSpec {
    pub side: Side,
    pub kind: OrderKind,
    pub qty: Qty,
    pub tif: TimeInForce,
    pub owner_index: u16,
}

/// One engine operation, addressed relative to the harness state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpTemplate {
    Submit(SubmitSpec),
    /// Cancel the `target % issued.len()`-th previously issued order
    /// (no-op when nothing was issued yet).
    Cancel {
        target: usize,
    },
    /// Modify a previously issued order, addressed like [`OpTemplate::Cancel`].
    Modify {
        target: usize,
        new_price: Option<Price>,
        new_remaining: Qty,
    },
}

impl SubmitSpec {
    /// Is this spec valid under `domain` (used as a Kani assumption)?
    #[must_use]
    pub fn is_valid(&self, domain: &Domain) -> bool {
        let qty_ok = !self.qty.is_zero() && self.qty.lots() <= domain.max_qty_lots;
        let owner_ok = self.owner_index < domain.owners;
        let kind_ok = match self.kind {
            OrderKind::Limit(p) => p.is_positive() && p.ticks() <= domain.max_price_ticks,
            OrderKind::Market => self.tif != TimeInForce::Gtc,
        };
        qty_ok && owner_ok && kind_ok
    }
}
