//! Contiguous price-band ladder book (the "HFT-style" layout).
//!
//! Levels live in a flat `Vec` indexed by `price_tick − base`, so touching a
//! level is pure pointer arithmetic — no tree walk, no rebalancing, no
//! allocator traffic per level. The trade-offs the benchmark quantifies:
//!
//! - O(1) level access for any in-band price vs `BTreeMap`'s O(log P);
//! - best-price maintenance scans toward worse prices when the touch
//!   empties — cheap when the book is dense near the touch (the realistic
//!   case Hawkes workloads produce), degenerate when it is sparse;
//! - prices outside the band force a recenter (amortized; counted in
//!   [`LadderBook::recenters`]).
//!
//! Semantics are identical to [`super::btree::BTreeBook`] by construction —
//! same intrusive slab FIFO levels — and pinned by the differential test
//! in `tests/differential.rs`, which drives all three books through the
//! same op sequences and compares snapshots at every step.

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

#[derive(Debug, Clone, Copy)]
struct Level {
    head: Option<usize>,
    tail: Option<usize>,
    total_lots: u128,
    count: u32,
}

impl Level {
    const EMPTY: Self = Self {
        head: None,
        tail: None,
        total_lots: 0,
        count: 0,
    };
}

/// One side's price band: `levels[i]` holds price tick `base + i`.
#[derive(Debug)]
struct Band {
    base: i64,
    levels: Vec<Level>,
    /// Index of the best occupied level (max for bids, min for asks).
    best: Option<usize>,
    /// Number of resting orders on this side.
    orders: usize,
}

impl Band {
    fn new() -> Self {
        Self {
            base: 0,
            levels: Vec::new(),
            best: None,
            orders: 0,
        }
    }

    fn idx_of(&self, price: Price) -> Option<usize> {
        let off = price.ticks().checked_sub(self.base)?;
        usize::try_from(off).ok().filter(|&i| i < self.levels.len())
    }

    /// Ensure `price` is inside the band, recentering/growing if needed.
    /// Returns the level index. `recenters` counts reallocation events.
    fn ensure(&mut self, price: Price, recenters: &mut u64) -> usize {
        const INITIAL_HALF_WIDTH: i64 = 512;
        let tick = price.ticks();
        if self.levels.is_empty() {
            self.base = (tick - INITIAL_HALF_WIDTH).max(1);
            self.levels = vec![Level::EMPTY; (2 * INITIAL_HALF_WIDTH) as usize];
        }
        loop {
            if let Some(idx) = self.idx_of(price) {
                return idx;
            }
            // Out of band: grow around the union of [old band ∪ tick] with
            // fresh margin, shifting existing levels to their new offsets.
            *recenters += 1;
            let old_lo = self.base;
            let old_hi = self.base + self.levels.len() as i64;
            let margin = (self.levels.len() as i64).max(INITIAL_HALF_WIDTH);
            let new_lo = (tick.min(old_lo) - margin).max(1);
            let new_hi = tick.max(old_hi - 1) + margin;
            let mut new_levels = vec![Level::EMPTY; (new_hi - new_lo + 1) as usize];
            let shift = (old_lo - new_lo) as usize;
            for (i, level) in self.levels.drain(..).enumerate() {
                new_levels[shift + i] = level;
            }
            self.levels = new_levels;
            self.base = new_lo;
            self.best = self.best.map(|b| b + shift);
        }
    }

    fn price_at(&self, idx: usize) -> Price {
        Price::new(self.base + idx as i64)
    }

    /// After emptying `idx` (previously the best), scan toward worse prices
    /// for the next occupied level.
    fn rescan_best_from(&mut self, idx: usize, side: Side) {
        debug_assert_eq!(self.best, Some(idx));
        match side {
            Side::Buy => {
                // Worse for bids = lower index.
                self.best = self.levels[..idx].iter().rposition(|level| level.count > 0);
            }
            Side::Sell => {
                self.best = self.levels[idx + 1..]
                    .iter()
                    .position(|level| level.count > 0)
                    .map(|off| idx + 1 + off);
            }
        }
    }

    fn on_insert(&mut self, idx: usize, side: Side) {
        self.orders += 1;
        let better = match (self.best, side) {
            (None, _) => true,
            (Some(b), Side::Buy) => idx > b,
            (Some(b), Side::Sell) => idx < b,
        };
        if better {
            self.best = Some(idx);
        }
    }

    fn on_remove(&mut self, idx: usize, side: Side) {
        self.orders -= 1;
        if self.levels[idx].count == 0 && self.best == Some(idx) {
            self.rescan_best_from(idx, side);
        }
    }
}

/// Contiguous price-band book. Same semantics as `BTreeBook`; different
/// constant factors — see the module docs and the comparative benches.
#[derive(Debug)]
pub struct LadderBook {
    slab: Slab<OrderNode>,
    index: FxHashMap<OrderId, usize>,
    bids: Band,
    asks: Band,
    /// Band reallocation events (perf observability, not semantics).
    pub recenters: u64,
}

impl Default for LadderBook {
    fn default() -> Self {
        Self::new()
    }
}

impl LadderBook {
    #[must_use]
    pub fn new() -> Self {
        Self {
            slab: Slab::new(),
            index: FxHashMap::default(),
            bids: Band::new(),
            asks: Band::new(),
            recenters: 0,
        }
    }

    fn band(&self, side: Side) -> &Band {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    /// Disjoint mutable borrows of the slab, one band, and the id index.
    fn parts(
        &mut self,
        side: Side,
    ) -> (
        &mut Slab<OrderNode>,
        &mut Band,
        &mut FxHashMap<OrderId, usize>,
    ) {
        let band = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        (&mut self.slab, band, &mut self.index)
    }

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

    fn key_of(&self, id: OrderId) -> Result<usize, BookError> {
        self.index.get(&id).copied().ok_or(BookError::NotFound(id))
    }

    /// Level index for a resting order's price; `Corrupt` if out of band
    /// (a resting order's price is always in band by construction).
    fn idx_for(&self, order: &RestingOrder) -> Result<usize, BookError> {
        self.band(order.side)
            .idx_of(order.price)
            .ok_or_else(|| BookError::Corrupt(format!("resting order {} out of band", order.id)))
    }
}

impl Book for LadderBook {
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
        let recenters = &mut self.recenters;
        let band = match order.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let idx = band.ensure(order.price, recenters);
        Self::push_tail(&mut self.slab, &mut band.levels[idx], key);
        band.on_insert(idx, order.side);
        Ok(())
    }

    fn best_price(&self, side: Side) -> Option<Price> {
        let band = self.band(side);
        band.best.map(|idx| band.price_at(idx))
    }

    fn front(&self, side: Side) -> Option<RestingOrder> {
        let band = self.band(side);
        let level = &band.levels[band.best?];
        level.head.map(|key| self.slab[key].order)
    }

    fn fill_front(&mut self, side: Side, qty: Qty) -> Result<Qty, BookError> {
        let (slab, band, index) = self.parts(side);
        let idx = band
            .best
            .ok_or_else(|| BookError::Invalid("fill_front on empty side".into()))?;
        let level = &mut band.levels[idx];
        let key = level
            .head
            .ok_or_else(|| BookError::Corrupt("empty best level".into()))?;
        let node = &mut slab[key];
        let new_remaining = node
            .order
            .remaining
            .checked_sub(qty)
            .map_err(|_| BookError::Invalid("fill exceeds maker remaining".into()))?;
        node.order.remaining = new_remaining;
        level.total_lots -= u128::from(qty.lots());
        if new_remaining.is_zero() {
            Self::unlink(slab, level, key);
            band.on_remove(idx, side);
            let id = slab[key].order.id;
            slab.remove(key);
            index.remove(&id);
        }
        Ok(new_remaining)
    }

    fn remove(&mut self, id: OrderId) -> Result<RestingOrder, BookError> {
        let key = self.key_of(id)?;
        let order = self.slab[key].order;
        let idx = self.idx_for(&order)?;
        let (slab, band, index) = self.parts(order.side);
        Self::unlink(slab, &mut band.levels[idx], key);
        band.on_remove(idx, order.side);
        slab.remove(key);
        index.remove(&order.id);
        Ok(order)
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
        let idx = self.idx_for(&order)?;
        let delta = order.remaining.lots() - new_remaining.lots();
        let (slab, band, _) = self.parts(order.side);
        band.levels[idx].total_lots -= u128::from(delta);
        slab[key].order.remaining = new_remaining;
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
        let idx = self.idx_for(&order)?;
        let (slab, band, _) = self.parts(order.side);
        let level = &mut band.levels[idx];
        Self::unlink(slab, level, key);
        slab[key].order.remaining = new_remaining;
        Self::push_tail(slab, level, key);
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
        let band = self.band(maker_side);
        let Some(best) = band.best else {
            return acc >= needed;
        };
        // Walk maker levels best-first: descending indices for bids,
        // ascending for asks.
        let indices: Box<dyn Iterator<Item = usize>> = match maker_side {
            Side::Buy => Box::new((0..=best).rev()),
            Side::Sell => Box::new(best..band.levels.len()),
        };
        for idx in indices {
            let level = &band.levels[idx];
            if level.count == 0 {
                continue;
            }
            let price = band.price_at(idx);
            if !crosses(taker_side, price, limit) {
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
        let band = self.band(side);
        let idx = band.idx_of(price)?;
        let level = &band.levels[idx];
        (level.count > 0).then_some((level.total_lots, level.count))
    }

    fn order_count(&self) -> usize {
        self.index.len()
    }

    fn snapshot(&self) -> BookSnapshot {
        let collect_side = |band: &Band, side: Side| {
            let mut out = Vec::new();
            let occupied: Box<dyn Iterator<Item = usize>> = match side {
                Side::Buy => Box::new((0..band.levels.len()).rev()),
                Side::Sell => Box::new(0..band.levels.len()),
            };
            for idx in occupied {
                let level = &band.levels[idx];
                if level.count == 0 {
                    continue;
                }
                let mut orders = Vec::with_capacity(level.count as usize);
                let mut cursor = level.head;
                while let Some(key) = cursor {
                    orders.push(self.slab[key].order);
                    cursor = self.slab[key].next;
                }
                out.push((band.price_at(idx), orders));
            }
            out
        };
        BookSnapshot {
            bids: collect_side(&self.bids, Side::Buy),
            asks: collect_side(&self.asks, Side::Sell),
        }
    }

    fn check_invariants(&self) -> Result<(), BookError> {
        for (band, side) in [(&self.bids, Side::Buy), (&self.asks, Side::Sell)] {
            let mut seen_orders = 0usize;
            let mut best_seen: Option<usize> = None;
            for (idx, level) in band.levels.iter().enumerate() {
                if level.count == 0 {
                    if level.head.is_some() || level.tail.is_some() || level.total_lots != 0 {
                        return Err(BookError::Corrupt("empty level with residue".into()));
                    }
                    continue;
                }
                // Ascending scan: for bids the best is the last occupied
                // (max) index; for asks the first (min).
                if best_seen.is_none() || side == Side::Buy {
                    best_seen = Some(idx);
                }
                let mut lots: u128 = 0;
                let mut count: u32 = 0;
                let mut cursor = level.head;
                let mut prev: Option<usize> = None;
                while let Some(key) = cursor {
                    let node = &self.slab[key];
                    if node.order.side != side || band.idx_of(node.order.price) != Some(idx) {
                        return Err(BookError::Corrupt("order on wrong level".into()));
                    }
                    if node.prev != prev {
                        return Err(BookError::Corrupt("broken prev link".into()));
                    }
                    if node.order.remaining.is_zero() {
                        return Err(BookError::Corrupt("zero-remaining resting order".into()));
                    }
                    lots += u128::from(node.order.remaining.lots());
                    count += 1;
                    prev = cursor;
                    cursor = node.next;
                }
                if level.tail != prev || lots != level.total_lots || count != level.count {
                    return Err(BookError::Corrupt("level accounting mismatch".into()));
                }
                seen_orders += count as usize;
            }
            if band.best != best_seen {
                return Err(BookError::Corrupt(format!(
                    "cached best {:?} != scanned best {best_seen:?}",
                    band.best
                )));
            }
            if band.orders != seen_orders {
                return Err(BookError::Corrupt("side order count mismatch".into()));
            }
        }
        // Cross-side: never crossed at rest.
        if let (Some(bid), Some(ask)) = (self.best_price(Side::Buy), self.best_price(Side::Sell))
            && bid >= ask
        {
            return Err(BookError::Corrupt("book crossed at rest".into()));
        }
        if self.index.len() != self.bids.orders + self.asks.orders {
            return Err(BookError::Corrupt("index/side count mismatch".into()));
        }
        Ok(())
    }
}
