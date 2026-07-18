//! Harness driving [`OpTemplate`] sequences through a [`MatchingEngine`].
//!
//! Shared by the proptest suites (randomized templates) and the Kani proof
//! harnesses (symbolic templates) — one applier, one operation semantics.

use sim_core::ops::{OpTemplate, SubmitSpec};
use sim_core::{EventSeq, EventSink, NewOrder, OrderId, OwnerId, Request, SimDuration, SimTime};

use crate::book::{Book, BookError};
use crate::engine::MatchingEngine;

/// Drives an engine through relative operation templates, assigning fresh
/// monotonic order ids and advancing virtual time by one microsecond per op.
#[derive(Debug)]
pub struct Harness<B: Book> {
    pub engine: MatchingEngine<B>,
    pub seq: EventSeq,
    pub ts: SimTime,
    /// Ids issued for submissions, in order (targets index into this).
    pub issued: Vec<OrderId>,
    next_id: u64,
}

impl<B: Book> Harness<B> {
    #[must_use]
    pub fn new(engine: MatchingEngine<B>) -> Self {
        Self {
            engine,
            seq: EventSeq::new(0),
            ts: SimTime::EPOCH,
            issued: Vec::new(),
            next_id: 0,
        }
    }

    /// Resolve a relative target to a previously issued id.
    #[must_use]
    pub fn resolve(&self, target: usize) -> Option<OrderId> {
        if self.issued.is_empty() {
            None
        } else {
            Some(self.issued[target % self.issued.len()])
        }
    }

    /// Apply one template; no-ops (targeting an empty history) return
    /// `Ok(false)`.
    pub fn apply(&mut self, op: &OpTemplate, sink: &mut dyn EventSink) -> Result<bool, BookError> {
        self.ts = self
            .ts
            .checked_add(SimDuration::from_micros(1))
            .expect("virtual clock overflow");
        let request = match *op {
            OpTemplate::Submit(SubmitSpec {
                side,
                kind,
                qty,
                tif,
                owner_index,
            }) => {
                let id = OrderId::new(self.next_id);
                self.next_id += 1;
                self.issued.push(id);
                Request::Submit(NewOrder {
                    id,
                    owner: OwnerId::new(owner_index),
                    symbol: self.engine.symbol(),
                    side,
                    kind,
                    qty,
                    tif,
                })
            }
            OpTemplate::Cancel { target } => match self.resolve(target) {
                Some(id) => Request::Cancel { id },
                None => return Ok(false),
            },
            OpTemplate::Modify {
                target,
                new_price,
                new_remaining,
            } => match self.resolve(target) {
                Some(id) => Request::Modify {
                    id,
                    new_price,
                    new_remaining,
                },
                None => return Ok(false),
            },
        };
        self.engine
            .process(self.ts, &mut self.seq, sink, request)
            .map(|()| true)
    }
}
