//! Shared test vocabulary: bounded domains and proptest generators.
//!
//! The same bounded domains serve two masters: proptest suites (randomized,
//! large domains) and Kani proof harnesses in `matching-engine` (symbolic,
//! tiny domains). Keeping the bounds here means the property tests and the
//! proofs demonstrably talk about the same operation model.

use proptest::prelude::*;

use crate::ids::{OrderId, OwnerId, SymbolId};
use crate::order::{NewOrder, OrderKind, Side, TimeInForce};
use crate::price::{Price, Qty};

/// Bounds for generated order domains.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    pub max_price_ticks: i64,
    pub max_qty_lots: u64,
    pub owners: u16,
}

impl Domain {
    /// Default proptest domain: small enough to collide constantly (which is
    /// what exercises matching), large enough to be interesting.
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

pub fn side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Buy), Just(Side::Sell)]
}

pub fn tif() -> impl Strategy<Value = TimeInForce> {
    prop_oneof![
        3 => Just(TimeInForce::Gtc),
        1 => Just(TimeInForce::Ioc),
        1 => Just(TimeInForce::Fok),
    ]
}

pub fn price(domain: Domain) -> impl Strategy<Value = Price> {
    (1..=domain.max_price_ticks).prop_map(Price::new)
}

pub fn qty(domain: Domain) -> impl Strategy<Value = Qty> {
    (1..=domain.max_qty_lots).prop_map(Qty::new)
}

pub fn owner(domain: Domain) -> impl Strategy<Value = OwnerId> {
    (0..domain.owners).prop_map(OwnerId::new)
}

pub fn order_kind(domain: Domain) -> impl Strategy<Value = OrderKind> {
    prop_oneof![
        4 => price(domain).prop_map(OrderKind::Limit),
        1 => Just(OrderKind::Market),
    ]
}

/// A new order with the given id (ids are assigned sequentially by callers so
/// generated sequences never contain accidental duplicates — duplicate-id
/// rejection is tested explicitly instead).
pub fn new_order(id: u64, symbol: SymbolId, domain: Domain) -> impl Strategy<Value = NewOrder> {
    (
        side(),
        order_kind(domain),
        qty(domain),
        tif(),
        owner(domain),
    )
        .prop_map(move |(side, kind, qty, tif, owner)| {
            // Market+Gtc is invalid by construction here; validation-path tests
            // construct it deliberately.
            let tif = match (kind, tif) {
                (OrderKind::Market, TimeInForce::Gtc) => TimeInForce::Ioc,
                _ => tif,
            };
            NewOrder {
                id: OrderId::new(id),
                owner,
                symbol,
                side,
                kind,
                qty,
                tif,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn generated_orders_are_valid(o in new_order(7, SymbolId::new(0), Domain::PROPTEST)) {
            prop_assert!(!o.qty.is_zero());
            if let OrderKind::Limit(p) = o.kind {
                prop_assert!(p.is_positive());
            }
            prop_assert!(!(matches!(o.kind, OrderKind::Market) && o.tif == TimeInForce::Gtc));
        }
    }
}
