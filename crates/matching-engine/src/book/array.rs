//! `ArrayBook`: a tiny, capacity-bounded book over fixed arrays.
//!
//! This exists for one reason: **Kani can exhaustively check it**. The
//! production `BTreeBook`'s heap structures blow up symbolic execution; this
//! book stores at most `CAP` orders in a flat array and finds priority order
//! by scanning (O(CAP) per query, O(CAP²) for feasibility — irrelevant at
//! CAP ≤ 8). The matching algorithm is generic over [`Book`], so proving
//! properties against `ArrayBook` proves the *algorithm*; the differential
//! test (`tests/differential.rs`) then pins `ArrayBook ≡ BTreeBook` on
//! identical operation sequences, transferring that confidence to the
//! production book.
//!
//! FIFO is encoded with a monotonically increasing insertion sequence per
//! slot: within a price level, lower `seq` = earlier arrival. `move_to_tail`
//! assigns a fresh sequence (greater than everything, in particular greater
//! than everything at the same price — which is all FIFO comparisons ever
//! consider).

use sim_core::{OrderId, Price, Qty, Side};

use super::{Book, BookError, BookSnapshot, RestingOrder, StpCount, crosses};

#[derive(Debug, Clone, Copy)]
struct Slot {
    order: RestingOrder,
    seq: u64,
}

/// Bounded order book for verification.
#[derive(Debug, Clone)]
pub struct ArrayBook<const CAP: usize> {
    slots: [Option<Slot>; CAP],
    next_seq: u64,
}

impl<const CAP: usize> Default for ArrayBook<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> ArrayBook<CAP> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [None; CAP],
            next_seq: 0,
        }
    }

    /// Is `a` strictly ahead of `b` in matching priority on `side`?
    /// (Better price first; equal price → earlier arrival first.)
    const fn ahead(side: Side, a: (i64, u64), b: (i64, u64)) -> bool {
        let (ap, aseq) = a;
        let (bp, bseq) = b;
        if ap == bp {
            aseq < bseq
        } else {
            match side {
                Side::Buy => ap > bp,
                Side::Sell => ap < bp,
            }
        }
    }

    /// Index of the highest-priority resting order on `side`, optionally
    /// restricted to orders strictly *behind* `after` in priority order.
    fn scan_front(&self, side: Side, after: Option<(i64, u64)>) -> Option<usize> {
        let mut best: Option<(usize, (i64, u64))> = None;
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(s) = slot else { continue };
            if s.order.side != side {
                continue;
            }
            let key = (s.order.price.ticks(), s.seq);
            if let Some(a) = after
                && !Self::ahead(side, a, key)
            {
                continue;
            }
            match best {
                Some((_, b)) if !Self::ahead(side, key, b) => {}
                _ => best = Some((i, key)),
            }
        }
        best.map(|(i, _)| i)
    }

    fn find(&self, id: OrderId) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.is_some_and(|s| s.order.id == id))
    }
}

impl<const CAP: usize> Book for ArrayBook<CAP> {
    fn insert(&mut self, order: RestingOrder) -> Result<(), BookError> {
        if order.remaining.is_zero() {
            return Err(BookError::Invalid("insert of zero-remaining order".into()));
        }
        if self.find(order.id).is_some() {
            return Err(BookError::Duplicate(order.id));
        }
        let Some(free) = self.slots.iter().position(Option::is_none) else {
            return Err(BookError::Invalid("ArrayBook capacity exhausted".into()));
        };
        self.slots[free] = Some(Slot {
            order,
            seq: self.next_seq,
        });
        self.next_seq += 1;
        Ok(())
    }

    fn best_price(&self, side: Side) -> Option<Price> {
        self.scan_front(side, None)
            .map(|i| self.slots[i].expect("scanned slot").order.price)
    }

    fn front(&self, side: Side) -> Option<RestingOrder> {
        self.scan_front(side, None)
            .map(|i| self.slots[i].expect("scanned slot").order)
    }

    fn fill_front(&mut self, side: Side, qty: Qty) -> Result<Qty, BookError> {
        let i = self
            .scan_front(side, None)
            .ok_or_else(|| BookError::Invalid("fill_front on empty side".into()))?;
        let slot = self.slots[i].as_mut().expect("scanned slot");
        let new_remaining = slot
            .order
            .remaining
            .checked_sub(qty)
            .map_err(|_| BookError::Invalid("fill exceeds maker remaining".into()))?;
        slot.order.remaining = new_remaining;
        if new_remaining.is_zero() {
            self.slots[i] = None;
        }
        Ok(new_remaining)
    }

    fn remove(&mut self, id: OrderId) -> Result<RestingOrder, BookError> {
        let i = self.find(id).ok_or(BookError::NotFound(id))?;
        let order = self.slots[i].expect("found slot").order;
        self.slots[i] = None;
        Ok(order)
    }

    fn get(&self, id: OrderId) -> Option<RestingOrder> {
        self.find(id)
            .map(|i| self.slots[i].expect("found slot").order)
    }

    fn reduce_in_place(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError> {
        let i = self.find(id).ok_or(BookError::NotFound(id))?;
        let slot = self.slots[i].as_mut().expect("found slot");
        if new_remaining.is_zero() || new_remaining >= slot.order.remaining {
            return Err(BookError::Invalid(
                "reduce_in_place requires 0 < new < current".into(),
            ));
        }
        slot.order.remaining = new_remaining;
        Ok(())
    }

    fn move_to_tail(&mut self, id: OrderId, new_remaining: Qty) -> Result<(), BookError> {
        let i = self.find(id).ok_or(BookError::NotFound(id))?;
        if new_remaining.is_zero() {
            return Err(BookError::Invalid(
                "move_to_tail with zero remaining".into(),
            ));
        }
        let slot = self.slots[i].as_mut().expect("found slot");
        slot.order.remaining = new_remaining;
        slot.seq = self.next_seq;
        self.next_seq += 1;
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
        let maker_side = taker_side.opposite();
        let mut acc: u128 = 0;
        let mut cursor: Option<(i64, u64)> = None;
        while let Some(i) = self.scan_front(maker_side, cursor) {
            let slot = self.slots[i].expect("scanned slot");
            if !crosses(taker_side, slot.order.price, limit) {
                break;
            }
            cursor = Some((slot.order.price.ticks(), slot.seq));
            match stp {
                StpCount::All => acc += u128::from(slot.order.remaining.lots()),
                StpCount::Skip(owner) => {
                    if slot.order.owner != owner {
                        acc += u128::from(slot.order.remaining.lots());
                    }
                }
                StpCount::StopAt(owner) => {
                    if slot.order.owner == owner {
                        return acc >= needed;
                    }
                    acc += u128::from(slot.order.remaining.lots());
                }
            }
            if acc >= needed {
                return true;
            }
        }
        acc >= needed
    }

    fn level_info(&self, side: Side, price: Price) -> Option<(u128, u32)> {
        let mut total: u128 = 0;
        let mut count: u32 = 0;
        for slot in self.slots.iter().flatten() {
            if slot.order.side == side && slot.order.price == price {
                total += u128::from(slot.order.remaining.lots());
                count += 1;
            }
        }
        (count > 0).then_some((total, count))
    }

    fn order_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    fn snapshot(&self) -> BookSnapshot {
        let side_view = |side: Side, best_first_desc: bool| {
            let mut entries: Vec<(i64, u64, RestingOrder)> = self
                .slots
                .iter()
                .flatten()
                .filter(|s| s.order.side == side)
                .map(|s| (s.order.price.ticks(), s.seq, s.order))
                .collect();
            entries.sort_by_key(|(price, seq, _)| {
                (if best_first_desc { -price } else { *price }, *seq)
            });
            let mut out: Vec<(Price, Vec<RestingOrder>)> = Vec::new();
            for (price, _, order) in entries {
                match out.last_mut() {
                    Some((p, orders)) if p.ticks() == price => orders.push(order),
                    _ => out.push((Price::new(price), vec![order])),
                }
            }
            out
        };
        BookSnapshot {
            bids: side_view(Side::Buy, true),
            asks: side_view(Side::Sell, false),
        }
    }

    fn check_invariants(&self) -> Result<(), BookError> {
        // Unique ids, unique seqs, positive remainings, uncrossed sides.
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(a) = slot else { continue };
            if a.order.remaining.is_zero() {
                return Err(BookError::Corrupt("zero-remaining order rests".into()));
            }
            for b in self.slots.iter().skip(i + 1).flatten() {
                if a.order.id == b.order.id {
                    return Err(BookError::Corrupt("duplicate id".into()));
                }
                if a.seq == b.seq {
                    return Err(BookError::Corrupt("duplicate seq".into()));
                }
            }
        }
        if let (Some(bid), Some(ask)) = (self.best_price(Side::Buy), self.best_price(Side::Sell))
            && bid >= ask
        {
            return Err(BookError::Corrupt("book crossed".into()));
        }
        Ok(())
    }
}
