//! Ogata thinning for multivariate exponential-kernel Hawkes.
//!
//! Because exponential kernels only decay between events, the total
//! intensity immediately after the last state change is a valid upper bound
//! until the next event — the classic O(1)-per-candidate thinning argument.
//! Excitation state is O(M²); no event history is stored.

use rand::Rng as _;
use sim_core::rng::Rng;

#[derive(Debug, Clone)]
pub struct HawkesProcess {
    /// Baseline intensities per dimension (events/sec).
    mu: Vec<f64>,
    /// α[m][n]: jump added to dim m's intensity by an event of dim n (1/sec).
    alpha: Vec<Vec<f64>>,
    /// Shared exponential decay rate (1/sec).
    beta: f64,
    /// Excitation per `(m, n)`: sum over past n-events of `alpha_{mn}
    /// exp(-beta (t - t_i))`.
    excitation: Vec<Vec<f64>>,
    /// Time of the last state update (seconds).
    last_t: f64,
}

impl HawkesProcess {
    /// Construct with a stability check: spectral radius of α/β must be < 1.
    pub fn new(mu: Vec<f64>, alpha: Vec<Vec<f64>>, beta: f64) -> Result<Self, String> {
        let dims = mu.len();
        if alpha.len() != dims || alpha.iter().any(|row| row.len() != dims) {
            return Err("alpha must be MxM matching mu".into());
        }
        if beta <= 0.0 {
            return Err("beta must be positive".into());
        }
        if mu.iter().any(|&m| m < 0.0) || alpha.iter().flatten().any(|&a| a < 0.0) {
            return Err("mu and alpha must be nonnegative".into());
        }
        let branching: Vec<Vec<f64>> = alpha
            .iter()
            .map(|row| row.iter().map(|a| a / beta).collect())
            .collect();
        let radius = super::spectral_radius(&branching);
        if radius >= 1.0 {
            return Err(format!("unstable Hawkes: spectral radius {radius:.4} >= 1"));
        }
        Ok(Self {
            excitation: vec![vec![0.0; dims]; dims],
            mu,
            alpha,
            beta,
            last_t: 0.0,
        })
    }

    #[must_use]
    pub fn dims(&self) -> usize {
        self.mu.len()
    }

    /// Intensity of dim `m` at the current state time.
    fn lambda(&self, m: usize) -> f64 {
        self.mu[m] + self.excitation[m].iter().sum::<f64>()
    }

    fn total_intensity(&self) -> f64 {
        (0..self.dims()).map(|m| self.lambda(m)).sum()
    }

    /// Decay excitation forward to absolute time `t` (seconds).
    fn advance_to(&mut self, t: f64) {
        let dt = t - self.last_t;
        if dt > 0.0 {
            let decay = libm::exp(-self.beta * dt);
            for row in &mut self.excitation {
                for s in row.iter_mut() {
                    *s *= decay;
                }
            }
            self.last_t = t;
        }
    }

    /// Next event after the current time: `(t_seconds, dimension)`.
    ///
    /// Deterministic given the RNG stream. Returns `None` only if the
    /// process is silent (all μ = 0 and no excitation).
    pub fn next_event(&mut self, rng: &mut Rng) -> Option<(f64, usize)> {
        loop {
            let bound = self.total_intensity();
            if bound <= 0.0 {
                return None;
            }
            // Candidate arrival from the bounding homogeneous process.
            let u: f64 = 1.0 - rng.random::<f64>();
            let dt = -libm::log(u) / bound;
            let t = self.last_t + dt;
            self.advance_to(t);
            let lambda_t = self.total_intensity();
            let accept: f64 = rng.random();
            if accept * bound <= lambda_t {
                // Pick the dimension proportionally to λ_m(t).
                let mut pick: f64 = rng.random::<f64>() * lambda_t;
                let mut dim = self.dims() - 1;
                for m in 0..self.dims() {
                    let lm = self.lambda(m);
                    if pick < lm {
                        dim = m;
                        break;
                    }
                    pick -= lm;
                }
                // Event of dim `dim` excites every m by α[m][dim].
                for m in 0..self.dims() {
                    self.excitation[m][dim] += self.alpha[m][dim];
                }
                return Some((t, dim));
            }
            // Rejected: state already decayed to t; bound tightened; retry.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::rng::{SimRng, StreamId};

    /// Empirical rate of a stable Hawkes must approach μ/(1−ρ) (stationary
    /// branching identity), and the same seed must reproduce exactly.
    #[test]
    fn stationary_rate_and_determinism() {
        let build = || {
            HawkesProcess::new(vec![0.5, 0.5], vec![vec![0.3, 0.1], vec![0.1, 0.3]], 1.0).unwrap()
        };
        let run = |seed: u64| {
            let mut rng = SimRng::new(seed).stream(StreamId::FLOW_SOURCE);
            let mut process = build();
            let mut count = 0u64;
            let mut last = 0.0;
            while let Some((t, _dim)) = process.next_event(&mut rng) {
                if t > 20_000.0 {
                    break;
                }
                last = t;
                count += 1;
            }
            (count, last)
        };
        let (count, last) = run(11);
        assert_eq!(run(11), (count, last), "same seed must reproduce");
        // Total stationary rate: sum over dims of (I - G)^-1 μ.
        // G = [[0.3,0.1],[0.1,0.3]] → symmetric; total = 2 * 0.5/(1-0.4) = 1.667/s.
        let rate = count as f64 / last;
        let expected = 2.0 * 0.5 / (1.0 - 0.4);
        assert!(
            (rate - expected).abs() / expected < 0.05,
            "empirical rate {rate:.3} vs stationary {expected:.3}"
        );
    }

    #[test]
    fn rejects_unstable_parameters() {
        assert!(HawkesProcess::new(vec![1.0], vec![vec![1.1]], 1.0).is_err());
        assert!(HawkesProcess::new(vec![1.0], vec![vec![0.5]], 0.0).is_err());
    }
}
