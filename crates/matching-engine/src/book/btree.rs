//! The production book: `BTreeMap` price ladder + slab-backed intrusive FIFO
//! levels + id index.
//!
//! Complexity: O(log P) to open a new price level, O(1) enqueue at an
//! existing level, O(1) cancel/modify by id (index lookup + doubly-linked
//! unlink), O(1) best-price access. Iteration order is fully deterministic
//! (`BTreeMap` is ordered; the id index is a point-lookup-only `FxHashMap`
//! and is never iterated).

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use sim_core::{OrderId, Price, Qty, Side};
use slab::Slab;

use super::{Book, BookError, BookSnapshot, RestingOrder, StpCount, crosses};

#[derive(Debug, Clone)]
struct OrderNode {
    order: RestingOrder,
    prev: Option<usize>,
    next: Option<usize>,
}

#[derive(Debug, Clone)]
struct Level {
    head: Option<usize>,
    tail: Option<usize>,
    total_lots: u128,
    count: u32,
}

impl Level {
    const fn empty() -> Self {
        Self {
            head: None,
            tail: None,
            total_lots: 0,
            count: 0,
        }
    }
}

/// Production order book.
#[derive(Debug, Default)]
pub struct BTreeBook {
    slab: Slab<OrderNode>,
    index: FxHashMap<OrderId, usize>,
    bids: BTreeMap<Price, Level>,
    asks: BTreeMap<Price, Level>,
}

impl BTreeBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ladder(&self, side: Side) -> &BTreeMap<Price, Level> {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    fn best_price_of(&self, side: Side) -> Option<Price> {
        match side {
            Side::Buy => self.bids.last_key_value().map(|(p, _)| *p),
            Side::Sell => self.asks.first_key_value().map(|(p, _)| *p),
        }
    }

    /// Link `key` at the tail of `level`.
    fn push_tail(slab: &mut Slab<OrderNode>, level: &mut Level, key: usize) {
        let remaining = u128::from(slab[key].order.remaining.lots());
        slab[key].prev = level.tail;
        slab[key].next = None;
        match level.tail {
            Some(tail) => slab[tail].next = Some(key),
            None => level.head = Some(key),
        }
        level.tail = Some(key);
        level.count += 1;
        level.total_lots += remaining;
    }

    /// Unlink `key` from `level` (does not touch the slab entry itself).
    fn unlink(slab: &mut Slab<OrderNode>, level: &mut Level, key: usize) {
        let (prev, next) = (slab[key].prev, slab[key].next);
        match prev {
            Some(p) => slab[p].next = next,
            None => level.head = next,
        }
        match next {
            Some(n) => slab[n].prev = prev,
            None => level.tail = prev,
        }
        slab[key].prev = None;
        slab[key].next = None;
        level.count -= 1;
        level.total_lots -= u128::from(slab[key].order.remaining.lots());
    }

    /// Remove a fully-processed order from level/ladder/slab/index.
    fn evict(&mut self, key: usize) -> Result<RestingOrder, BookError> {
        let order = self.slab[key].order;
        let ladder = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = ladder.get_mut(&order.price).ok_or_else(|| {
            BookError::Corrupt(format!("no level for resting order {}", order.id))
        })?;
        Self::unlink(&mut self.slab, level, key);
        if level.count == 0 {
            ladder.remove(&order.price);
        }
        self.slab.remove(key);
        self.index.remove(&order.id);
        Ok(order)
    }

    fn key_of(&self, id: OrderId) -> Result<usize, BookError> {
        self.index.get(&id).copied().ok_or(BookError::NotFound(id))
    }
}

impl Book for BTreeBook {
    fn insert(&mut self, order: RestingOrder) -> Result<(), BookError> {
        if order.remaining.is_zero() {
            return Err(BookError::Invalid(format!(
                "insert of zero-remaining order {}",
                order.id
            )));
        }
        if self.index.contains_key(&order.id) {
            return Err(BookError::Duplicate(order.id));
        }
        let key = self.slab.insert(OrderNode {
            order,
            prev: None,
            next: None,
        });
        self.index.insert(order.id, key);
        let ladder = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = ladder.entry(order.price).or_insert_with(Level::empty);
        Self::push_tail(&mut self.slab, level, key);
        Ok(())
    }

    fn best_price(&self, side: Side) -> Option<Price> {
        self.best_price_of(side)
    }

    fn front(&self, side: Side) -> Option<RestingOrder> {
        let best = self.best_price_of(side)?;
        let level = self.ladder(side).get(&best)?;
        level.head.map(|key| self.slab[key].order)
    }

    fn fill_front(&mut self, side: Side, qty: Qty) -> Result<Qty, BookError> {
        let best = self
            .best_price_of(side)
            .ok_or_else(|| BookError::Invalid("fill_front on empty side".into()))?;
        let ladder = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = ladder
            .get_mut(&best)
            .ok_or_else(|| BookError::Corrupt("best level vanished".into()))?;
        let key = level
            .head
            .ok_or_else(|| BookError::Corrupt("empty level in ladder".into()))?;
        let node = &mut self.slab[key];
        let new_remaining = node
            .order
            .remaining
            .checked_sub(qty)
            .map_err(|_| BookError::Invalid("fill exceeds maker remaining".into()))?;
        node.order.remaining = new_remaining;
        level.total_lots -= u128::from(qty.lots());
        if new_remaining.is_zero() {
            Self::unlink(&mut self.slab, level, key);
            if level.count == 0 {
                ladder.remove(&best);
            }
            let id = self.slab[key].order.id;
            self.slab.remove(key);
            self.index.remove(&id);
        }
        Ok(new_remaining)
    }

    fn remove(&mut self, id: OrderId) -> Result<RestingOrder, BookError> {
        let key = self.key_of(id)?;
        self.evict(key)
    }

    fn get(&self, id: OrderId) -> Option<RestingOrder> {
        self.index.get(&id).map(|&key| self.slab[key].order)
    }

    fn reduce_in_place(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError> {
        let key = self.key_of(id)?;
        let order = self.slab[key].order;
        if new_remaining.is_zero() || new_remaining >= order.remaining {
            return Err(BookError::Invalid(
                "reduce_in_place requires 0 < new < current".into(),
            ));
        }
        let ladder = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = ladder
            .get_mut(&order.price)
            .ok_or_else(|| BookError::Corrupt("no level for resting order".into()))?;
        let delta = order.remaining.lots() - new_remaining.lots();
        level.total_lots -= u128::from(delta);
        self.slab[key].order.remaining = new_remaining;
        Ok(())
    }

    fn move_to_tail(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError> {
        let key = self.key_of(id)?;
        let order = self.slab[key].order;
        if new_remaining.is_zero() {
            return Err(BookError::Invalid(
                "move_to_tail with zero remaining".into(),
            ));
        }
        let ladder = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = ladder
            .get_mut(&order.price)
            .ok_or_else(|| BookError::Corrupt("no level for resting order".into()))?;
        Self::unlink(&mut self.slab, level, key);
        self.slab[key].order.remaining = new_remaining;
        Self::push_tail(&mut self.slab, level, key);
        Ok(())
    }

    fn fok_feasible(
        &self,
        taker_side: Side,
        limit: Option<Price>,
        needed: Qty,
        stp: StpCount,
    ) -> bool {
        let needed = u128::from(needed.lots());
        let mut acc: u128 = 0;
        let maker_side = taker_side.opposite();
        // Iterate maker levels best-first.
        let levels: Box<dyn Iterator<Item = (&Price, &Level)>> = match maker_side {
            Side::Buy => Box::new(self.bids.iter().rev()),
            Side::Sell => Box::new(self.asks.iter()),
        };
        for (price, level) in levels {
            if !crosses(taker_side, *price, limit) {
                break;
            }
            let mut cursor = level.head;
            while let Some(key) = cursor {
                let node = &self.slab[key];
                match stp {
                    StpCount::All => acc += u128::from(node.order.remaining.lots()),
                    StpCount::Skip(owner) => {
                        if node.order.owner != owner {
                            acc += u128::from(node.order.remaining.lots());
                        }
                    }
                    StpCount::StopAt(owner) => {
                        if node.order.owner == owner {
                            return acc >= needed;
                        }
                        acc += u128::from(node.order.remaining.lots());
                    }
                }
                if acc >= needed {
                    return true;
                }
                cursor = node.next;
            }
        }
        acc >= needed
    }

    fn level_info(&self, side: Side, price: Price) -> Option<(u128, u32)> {
        self.ladder(side)
            .get(&price)
            .map(|level| (level.total_lots, level.count))
    }

    fn order_count(&self) -> usize {
        self.index.len()
    }

    fn snapshot(&self) -> BookSnapshot {
        let collect_side = |ladder: &BTreeMap<Price, Level>, best_first_rev: bool| {
            let mut out = Vec::with_capacity(ladder.len());
            let entries: Box<dyn Iterator<Item = (&Price, &Level)>> = if best_first_rev {
                Box::new(ladder.iter().rev())
            } else {
                Box::new(ladder.iter())
            };
            for (price, level) in entries {
                let mut orders = Vec::with_capacity(level.count as usize);
                let mut cursor = level.head;
                while let Some(key) = cursor {
                    orders.push(self.slab[key].order);
                    cursor = self.slab[key].next;
                }
                out.push((*price, orders));
            }
            out
        };
        BookSnapshot {
            bids: collect_side(&self.bids, true),
            asks: collect_side(&self.asks, false),
        }
    }

    fn check_invariants(&self) -> Result<(), BookError> {
        let corrupt = |msg: String| Err(BookError::Corrupt(msg));

        // Uncrossed at rest.
        if let (Some(bid), Some(ask)) = (
            self.best_price_of(Side::Buy),
            self.best_price_of(Side::Sell),
        ) && bid >= ask
        {
            return corrupt(format!("book crossed: bid {bid} >= ask {ask}"));
        }

        let mut seen_nodes = 0usize;
        for (side, ladder) in [(Side::Buy, &self.bids), (Side::Sell, &self.asks)] {
            for (price, level) in ladder {
                if level.count == 0 || level.head.is_none() || level.tail.is_none() {
                    return corrupt(format!("empty level lingers at {price}"));
                }
                let mut cursor = level.head;
                let mut prev: Option<usize> = None;
                let mut count = 0u32;
                let mut total: u128 = 0;
                while let Some(key) = cursor {
                    let node = &self.slab[key];
                    if node.prev != prev {
                        return corrupt(format!("broken prev link at {price}"));
                    }
                    if node.order.side != side || node.order.price != *price {
                        return corrupt(format!(
                            "order {} misfiled at {price}/{side:?}",
                            node.order.id
                        ));
                    }
                    if node.order.remaining.is_zero() {
                        return corrupt(format!("zero-remaining order {} rests", node.order.id));
                    }
                    match self.index.get(&node.order.id) {
                        Some(&k) if k == key => {}
                        _ => return corrupt(format!("index mismatch for {}", node.order.id)),
                    }
                    count += 1;
                    total += u128::from(node.order.remaining.lots());
                    prev = cursor;
                    cursor = node.next;
                }
                if level.tail != prev {
                    return corrupt(format!("tail mismatch at {price}"));
                }
                if count != level.count || total != level.total_lots {
                    return corrupt(format!(
                        "level accounting mismatch at {price}: count {count}/{}, total {total}/{}",
                        level.count, level.total_lots
                    ));
                }
                seen_nodes += count as usize;
            }
        }
        if seen_nodes != self.index.len() || seen_nodes != self.slab.len() {
            return corrupt(format!(
                "node accounting mismatch: reachable {seen_nodes}, index {}, slab {}",
                self.index.len(),
                self.slab.len()
            ));
        }
        Ok(())
    }
}
