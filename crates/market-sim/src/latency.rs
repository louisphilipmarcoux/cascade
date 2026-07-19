//! Latency models — every link in the simulation gets one.
//!
//! All transcendentals go through `libm` (not `std`), so sampled values are
//! bit-identical across platforms — part of the cross-platform determinism
//! effort (see docs/determinism.md).

use rand::Rng as _;
use serde::{Deserialize, Serialize};
use sim_core::SimDuration;
use sim_core::rng::Rng;

/// Standard normal via Box–Muller with portable `libm` transcendentals.
pub(crate) fn standard_normal(rng: &mut Rng) -> f64 {
    // u1 in (0, 1] to keep log finite.
    let u1: f64 = 1.0 - rng.random::<f64>();
    let u2: f64 = rng.random();
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * core::f64::consts::PI * u2)
}

/// Exponential with the given rate (events per nanosecond scale-free: pass
/// mean in ns and get ns back).
pub(crate) fn exponential_ns(rng: &mut Rng, mean_ns: f64) -> f64 {
    let u: f64 = 1.0 - rng.random::<f64>();
    -libm::log(u) * mean_ns
}

/// A latency model samples a nonnegative delay for one message.
pub trait LatencyModel: std::fmt::Debug + Send {
    fn sample(&mut self, rng: &mut Rng) -> SimDuration;
}

/// Configuration for latency models (scenario-file vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LatencySpec {
    Zero,
    Constant {
        nanos: u64,
    },
    /// Log-normal around `median_ns` with log-scale `sigma`, floored at
    /// `min_ns` (a message cannot arrive before it was sent).
    LogNormal {
        median_ns: u64,
        sigma: f64,
        min_ns: u64,
    },
    /// Base model plus seeded delay spikes: with probability `spike_p`, the
    /// sampled delay is multiplied by `spike_mult` (fault-injection hook).
    Spiky {
        median_ns: u64,
        sigma: f64,
        min_ns: u64,
        spike_p: f64,
        spike_mult: f64,
    },
}

impl LatencySpec {
    #[must_use]
    pub fn build(&self) -> Box<dyn LatencyModel> {
        match *self {
            Self::Zero => Box::new(Constant(SimDuration::ZERO)),
            Self::Constant { nanos } => Box::new(Constant(SimDuration::from_nanos(nanos as i64))),
            Self::LogNormal {
                median_ns,
                sigma,
                min_ns,
            } => Box::new(LogNormal {
                median_ns: median_ns as f64,
                sigma,
                min_ns,
            }),
            Self::Spiky {
                median_ns,
                sigma,
                min_ns,
                spike_p,
                spike_mult,
            } => Box::new(Spiky {
                base: LogNormal {
                    median_ns: median_ns as f64,
                    sigma,
                    min_ns,
                },
                spike_p,
                spike_mult,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Constant(pub SimDuration);

impl LatencyModel for Constant {
    fn sample(&mut self, _rng: &mut Rng) -> SimDuration {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LogNormal {
    median_ns: f64,
    sigma: f64,
    min_ns: u64,
}

impl LogNormal {
    fn raw_sample(&self, rng: &mut Rng) -> f64 {
        // exp(ln(median) + sigma·Z): median-parameterized log-normal.
        self.median_ns * libm::exp(self.sigma * standard_normal(rng))
    }
}

impl LatencyModel for LogNormal {
    fn sample(&mut self, rng: &mut Rng) -> SimDuration {
        let ns = self.raw_sample(rng).max(self.min_ns as f64);
        // Cap at ~292 years; a latency model cannot overflow the clock.
        SimDuration::from_nanos(ns.min(i64::MAX as f64) as i64)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Spiky {
    base: LogNormal,
    spike_p: f64,
    spike_mult: f64,
}

impl LatencyModel for Spiky {
    fn sample(&mut self, rng: &mut Rng) -> SimDuration {
        let mut ns = self.base.raw_sample(rng).max(self.base.min_ns as f64);
        if rng.random::<f64>() < self.spike_p {
            ns *= self.spike_mult;
        }
        SimDuration::from_nanos(ns.min(i64::MAX as f64) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::rng::{SimRng, StreamId};

    #[test]
    fn lognormal_is_deterministic_and_floored() {
        let spec = LatencySpec::LogNormal {
            median_ns: 200_000,
            sigma: 0.4,
            min_ns: 20_000,
        };
        let sample_n = |n: usize| -> Vec<i64> {
            let mut rng = SimRng::new(9).stream(StreamId::LATENCY_ORDER);
            let mut model = spec.build();
            (0..n).map(|_| model.sample(&mut rng).nanos()).collect()
        };
        let a = sample_n(1000);
        let b = sample_n(1000);
        assert_eq!(a, b, "same seed, same stream ⇒ same delays");
        assert!(a.iter().all(|&ns| ns >= 20_000), "floor respected");
        // Median should be loosely near 200µs (sanity, wide tolerance).
        let mut sorted = a.clone();
        sorted.sort_unstable();
        let median = sorted[sorted.len() / 2];
        assert!((100_000..400_000).contains(&median), "median {median} off");
    }

    #[test]
    fn spiky_produces_occasional_spikes() {
        let spec = LatencySpec::Spiky {
            median_ns: 100_000,
            sigma: 0.1,
            min_ns: 1,
            spike_p: 0.05,
            spike_mult: 50.0,
        };
        let mut rng = SimRng::new(3).stream(StreamId::FAULTS);
        let mut model = spec.build();
        let samples: Vec<i64> = (0..2000).map(|_| model.sample(&mut rng).nanos()).collect();
        let spikes = samples.iter().filter(|&&ns| ns > 1_000_000).count();
        assert!(spikes > 20, "expected spikes, got {spikes}");
        assert!(spikes < 400, "too many spikes: {spikes}");
    }
}
