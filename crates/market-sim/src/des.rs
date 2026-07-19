//! Discrete-event scheduler with a virtual clock.
//!
//! Time advances **only** by popping this queue. Ties on the timestamp are
//! broken by a monotonic push sequence, making pop order — and therefore the
//! entire simulation — fully deterministic (P22).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use sim_core::{EventRecord, Request, SimTime, SymbolId};

/// Identifies a flow source within a simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u32);

/// Identifies an external actor (strategy) within a simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorId(pub u32);

/// Everything that can be scheduled.
#[derive(Debug, Clone)]
pub enum SimEvent {
    /// Ask a flow source for its next intent.
    SourceWake { source: SourceId },
    /// A request arriving at an engine (after order-path latency).
    EngineRequest { symbol: SymbolId, request: Request },
    /// A market-data record arriving at a subscriber (after md latency).
    MarketData {
        subscriber: ActorId,
        record: EventRecord,
    },
    /// A timer an actor asked for.
    Timer { owner: ActorId, token: u64 },
    /// Periodic equity sampling mark.
    SampleTick,
    /// Hard end of simulation.
    EndOfSim,
}

#[derive(Debug)]
struct Scheduled {
    at: SimTime,
    seq: u64,
    event: SimEvent,
}

impl PartialEq for Scheduled {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}
impl Eq for Scheduled {}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Scheduled {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.at, self.seq).cmp(&(other.at, other.seq))
    }
}

/// The deterministic event queue.
#[derive(Debug, Default)]
pub struct Scheduler {
    heap: BinaryHeap<Reverse<Scheduled>>,
    push_seq: u64,
    now: SimTime,
}

impl Scheduler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current virtual time (the timestamp of the last popped event).
    #[must_use]
    pub const fn now(&self) -> SimTime {
        self.now
    }

    /// Schedule `event` at absolute time `at`. Scheduling in the past is a
    /// logic error and is clamped to `now` (documented, asserted in tests).
    pub fn schedule(&mut self, at: SimTime, event: SimEvent) {
        let at = at.max(self.now);
        self.heap.push(Reverse(Scheduled {
            at,
            seq: self.push_seq,
            event,
        }));
        self.push_seq += 1;
    }

    /// Pop the next event, advancing the clock.
    pub fn pop(&mut self) -> Option<(SimTime, SimEvent)> {
        let Reverse(scheduled) = self.heap.pop()?;
        debug_assert!(scheduled.at >= self.now, "time went backwards");
        self.now = scheduled.at;
        Some((scheduled.at, scheduled.event))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_in_time_order_with_fifo_ties() {
        let mut s = Scheduler::new();
        s.schedule(
            SimTime::from_millis(5),
            SimEvent::Timer {
                owner: ActorId(0),
                token: 0,
            },
        );
        s.schedule(
            SimTime::from_millis(1),
            SimEvent::Timer {
                owner: ActorId(0),
                token: 1,
            },
        );
        s.schedule(
            SimTime::from_millis(5),
            SimEvent::Timer {
                owner: ActorId(0),
                token: 2,
            },
        );
        s.schedule(
            SimTime::from_millis(3),
            SimEvent::Timer {
                owner: ActorId(0),
                token: 3,
            },
        );

        let tokens: Vec<u64> = std::iter::from_fn(|| s.pop())
            .map(|(_, e)| match e {
                SimEvent::Timer { token, .. } => token,
                _ => unreachable!(),
            })
            .collect();
        // 1ms, 3ms, then the two 5ms events in push (FIFO) order.
        assert_eq!(tokens, vec![1, 3, 0, 2]);
    }

    #[test]
    fn clock_never_goes_backwards() {
        let mut s = Scheduler::new();
        s.schedule(SimTime::from_millis(10), SimEvent::EndOfSim);
        let _ = s.pop();
        assert_eq!(s.now(), SimTime::from_millis(10));
        // Scheduling in the past clamps to now.
        s.schedule(SimTime::from_millis(1), SimEvent::EndOfSim);
        let (at, _) = s.pop().unwrap();
        assert_eq!(at, SimTime::from_millis(10));
    }
}
