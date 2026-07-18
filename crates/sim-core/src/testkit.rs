//! proptest generators over the shared operation model ([`crate::ops`]).
//!
//! The Kani harnesses in `matching-engine` construct the same
//! [`OpTemplate`]s symbolically; these strategies construct them randomly.

use proptest::prelude::*;

use crate::ids::{OrderId, OwnerId, SymbolId};
use crate::ops::{Domain, OpTemplate, SubmitSpec};
use crate::order::{NewOrder, OrderKind, Side, TimeInForce};
use crate::price::{Price, Qty};

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

pub fn submit_spec(domain: Domain) -> impl Strategy<Value = SubmitSpec> {
    (
        side(),
        order_kind(domain),
        qty(domain),
        tif(),
        0..domain.owners,
    )
        .prop_map(move |(side, kind, qty, tif, owner_index)| {
            // Market+Gtc is invalid by construction here; validation-path
            // tests construct it deliberately.
            let tif = match (kind, tif) {
                (OrderKind::Market, TimeInForce::Gtc) => TimeInForce::Ioc,
                _ => tif,
            };
            SubmitSpec {
                side,
                kind,
                qty,
                tif,
                owner_index,
            }
        })
}

pub fn op_template(domain: Domain) -> impl Strategy<Value = OpTemplate> {
    prop_oneof![
        5 => submit_spec(domain).prop_map(OpTemplate::Submit),
        2 => any::<usize>().prop_map(|target| OpTemplate::Cancel { target }),
        2 => (any::<usize>(), proptest::option::of(price(domain)), qty(domain)).prop_map(
            |(target, new_price, new_remaining)| OpTemplate::Modify {
                target,
                new_price,
                new_remaining,
            }
        ),
    ]
}

/// A sequence of operations (the workhorse input of the property suites).
pub fn op_sequence(domain: Domain, max_len: usize) -> impl Strategy<Value = Vec<OpTemplate>> {
    proptest::collection::vec(op_template(domain), 0..=max_len)
}

/// A new order with the given id (ids are assigned sequentially by callers so
/// generated sequences never contain accidental duplicates — duplicate-id
/// rejection is tested explicitly instead).
pub fn new_order(id: u64, symbol: SymbolId, domain: Domain) -> impl Strategy<Value = NewOrder> {
    submit_spec(domain).prop_map(move |spec| NewOrder {
        id: OrderId::new(id),
        owner: OwnerId::new(spec.owner_index),
        symbol,
        side: spec.side,
        kind: spec.kind,
        qty: spec.qty,
        tif: spec.tif,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn generated_specs_are_valid(spec in submit_spec(Domain::PROPTEST)) {
            prop_assert!(spec.is_valid(&Domain::PROPTEST));
        }

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
