//! Seeded randomness with per-component substreams.
//!
//! One master seed drives the whole run. Every component (each flow source,
//! each agent, each latency link) draws from its **own** `ChaCha12` substream,
//! derived from `(master_seed, stream_id)`. Consequences:
//!
//! - adding or removing a component never perturbs any other component's
//!   random sequence, which is what makes paired-seed A/B strategy
//!   comparisons valid variance reduction;
//! - replays are exact: the same `(scenario, seed)` reproduces every draw.

use rand_chacha::ChaCha12Rng;
use rand_chacha::rand_core::SeedableRng;
use serde::{Deserialize, Serialize};

/// The concrete RNG used everywhere in the simulation.
pub type Rng = ChaCha12Rng;

/// Well-known substream ids. Components created dynamically allocate ids
/// above [`StreamId::DYNAMIC_BASE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub u64);

impl StreamId {
    pub const FLOW_SOURCE: Self = Self(1);
    pub const LATENCY_ORDER: Self = Self(2);
    pub const LATENCY_MARKET_DATA: Self = Self(3);
    pub const FAULTS: Self = Self(4);
    pub const DYNAMIC_BASE: u64 = 1024;
}

/// Master seed for a run; hands out independent substreams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimRng {
    master_seed: u64,
}

impl SimRng {
    #[must_use]
    pub const fn new(master_seed: u64) -> Self {
        Self { master_seed }
    }

    #[must_use]
    pub const fn master_seed(&self) -> u64 {
        self.master_seed
    }

    /// An independent RNG for the given substream.
    #[must_use]
    pub fn stream(&self, id: StreamId) -> Rng {
        let mut rng = ChaCha12Rng::seed_from_u64(self.master_seed);
        rng.set_stream(id.0);
        rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng as _;

    #[test]
    fn same_seed_same_stream_reproduces() {
        let a = SimRng::new(42);
        let b = SimRng::new(42);
        let xs: Vec<u64> = a
            .stream(StreamId::FLOW_SOURCE)
            .random_iter()
            .take(8)
            .collect();
        let ys: Vec<u64> = b
            .stream(StreamId::FLOW_SOURCE)
            .random_iter()
            .take(8)
            .collect();
        assert_eq!(xs, ys);
    }

    #[test]
    fn different_streams_are_independent() {
        let rng = SimRng::new(42);
        let xs: Vec<u64> = rng
            .stream(StreamId::FLOW_SOURCE)
            .random_iter()
            .take(8)
            .collect();
        let ys: Vec<u64> = rng
            .stream(StreamId::LATENCY_ORDER)
            .random_iter()
            .take(8)
            .collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn different_seeds_differ() {
        let xs: Vec<u64> = SimRng::new(1)
            .stream(StreamId::FLOW_SOURCE)
            .random_iter()
            .take(8)
            .collect();
        let ys: Vec<u64> = SimRng::new(2)
            .stream(StreamId::FLOW_SOURCE)
            .random_iter()
            .take(8)
            .collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn draws_are_stable_across_releases() {
        // Golden values: if this test ever fails, the RNG stack changed its
        // output and every committed golden hash in the repo must be
        // regenerated in the same commit (documented in docs/determinism.md).
        let mut rng = SimRng::new(42).stream(StreamId::FLOW_SOURCE);
        let draws: [u64; 3] = core::array::from_fn(|_| rng.random());
        assert_eq!(
            draws,
            [
                5_254_710_881_988_635_745,
                8_581_247_840_786_457_729,
                319_819_223_319_399_764
            ]
        );
    }
}
