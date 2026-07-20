//! Exponential-kernel Hawkes point-MLE (M dimensions, shared β).
//!
//! Log-likelihood over `[0, T]` with the standard recursion: for each dim
//! `m`,
//!
//! `LL_m = -mu_m T - sum_n (alpha_{mn}/beta) sum_i (1 - exp(-beta(T - t_i^n)))
//!         + sum_k log(mu_m + sum_n alpha_{mn} R_{mn}(k))`
//!
//! where `R_{mn}(k)` accumulates decayed excitation with O(N·M²) total work
//! (no history rescans). Optimization runs over log-transformed parameters
//! (positivity by construction) with a **deterministic** Nelder–Mead
//! (fixed simplex construction, fixed multi-start grid, no randomness) plus
//! a smooth stability penalty as ρ(α/β) → 1.
//!
//! The Python research layer implements the same likelihood independently
//! (scipy L-BFGS-B); a cross-implementation agreement test keeps both
//! honest. Bayesian credible intervals are a Stage-2 addition on the Python
//! side.

use serde::{Deserialize, Serialize};

use super::spectral_radius;

/// Events per dimension: strictly increasing times in seconds.
pub type Events = Vec<Vec<f64>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitResult {
    pub mu: Vec<f64>,
    pub alpha: Vec<Vec<f64>>,
    pub beta: f64,
    pub log_likelihood: f64,
    pub aic: f64,
    pub branching_matrix: Vec<Vec<f64>>,
    pub spectral_radius: f64,
    pub n_events: usize,
    pub t_span: f64,
    /// Log-likelihood of the best-fit homogeneous Poisson (per-dim MLE
    /// rate), for the likelihood-ratio story.
    pub poisson_log_likelihood: f64,
}

/// Merge per-dim event lists into a strictly-increasing (time, dim) stream.
///
/// Real tapes carry **tied timestamps** — Binance's Jan-2024 aggTrades are
/// millisecond-resolution, so many trades share an exact time. Coincident
/// events make `e^{−β·0}=1` for *any* β, opening a likelihood degeneracy the
/// optimizer drives to β → ∞ (a delta-kernel that "explains" simultaneity).
/// Sub-resolution timing is genuinely unobserved, so we spread runs of tied
/// events **deterministically** across the interval up to the next distinct
/// time (or a tiny default at the tail), yielding strictly increasing times.
/// Returns the stream and the smallest spacing introduced — the resolution
/// past which β is not identifiable, used to cap β in the objective.
fn merged(events: &Events) -> (Vec<(f64, usize)>, f64) {
    let mut all: Vec<(f64, usize)> = events
        .iter()
        .enumerate()
        .flat_map(|(dim, ts)| ts.iter().map(move |&t| (t, dim)))
        .collect();
    all.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite times"));

    // Default sub-resolution spacing when a tie run reaches the tape's end.
    let span = all.last().map_or(1.0, |(t, _)| *t) - all.first().map_or(0.0, |(t, _)| *t);
    let default_gap = (span / all.len().max(1) as f64).max(1e-9);

    let mut min_spacing = f64::INFINITY;
    let mut i = 0;
    while i < all.len() {
        let t = all[i].0;
        let mut j = i + 1;
        while j < all.len() && all[j].0 <= t {
            j += 1;
        }
        let run = j - i;
        if run > 1 {
            // Spread [i, j) uniformly over [t, next_distinct) (or default).
            let next = if j < all.len() {
                all[j].0
            } else {
                t + default_gap
            };
            let step = (next - t) / run as f64;
            for (k, item) in all[i..j].iter_mut().enumerate() {
                item.0 = t + step * k as f64;
            }
            min_spacing = min_spacing.min(step.max(1e-12));
        } else if j < all.len() {
            min_spacing = min_spacing.min((all[j].0 - t).max(1e-12));
        }
        i = j;
    }
    if !min_spacing.is_finite() {
        min_spacing = default_gap;
    }
    (all, min_spacing)
}

/// Negative log-likelihood for parameters given pre-merged events.
///
/// `theta` layout: `[ln mu_0..M, ln alpha_00..MM (row major), ln beta]`.
/// `beta_max` is the identifiability ceiling from the data's time resolution
/// (a kernel decaying faster than the finest spacing is unobservable).
fn negative_ll(
    theta: &[f64],
    stream: &[(f64, usize)],
    dims: usize,
    t_end: f64,
    beta_max: f64,
) -> f64 {
    let mu: Vec<f64> = theta[..dims].iter().map(|&x| libm::exp(x)).collect();
    let alpha: Vec<Vec<f64>> = (0..dims)
        .map(|m| {
            (0..dims)
                .map(|n| libm::exp(theta[dims + m * dims + n]))
                .collect()
        })
        .collect();
    let beta = libm::exp(theta[dims + dims * dims]);

    // β past the resolution ceiling is unidentifiable: hard wall.
    if beta > beta_max {
        return 1e12 + beta;
    }
    // Smooth stability barrier as ρ → 1 (and a hard wall beyond).
    let branching: Vec<Vec<f64>> = alpha
        .iter()
        .map(|row| row.iter().map(|a| a / beta).collect())
        .collect();
    let rho = spectral_radius(&branching);
    if rho >= 0.999 {
        return 1e12 + rho * 1e6;
    }
    let penalty = if rho > 0.95 {
        1e4 * (rho - 0.95).powi(2)
    } else {
        0.0
    };

    // R[m][n]: decayed excitation sums; log-intensity term accumulates as we
    // sweep the merged stream once.
    let mut excitation = vec![vec![0.0f64; dims]; dims];
    let mut last_t = 0.0f64;
    let mut log_term = 0.0f64;
    // Compensator pieces: Σ_n (α_{mn}/β) Σ_{t_i^n} (1 − e^{−β(T−t_i^n)}).
    let mut compensator = 0.0f64;

    for &(t, dim) in stream {
        let decay = libm::exp(-beta * (t - last_t));
        for row in &mut excitation {
            for s in row.iter_mut() {
                *s *= decay;
            }
        }
        last_t = t;
        let intensity = mu[dim] + excitation[dim].iter().sum::<f64>();
        if intensity <= 0.0 {
            return 1e12;
        }
        log_term += libm::log(intensity);
        let tail = 1.0 - libm::exp(-beta * (t_end - t));
        for (row, alpha_row) in excitation.iter_mut().zip(&alpha) {
            row[dim] += alpha_row[dim];
            compensator += alpha_row[dim] / beta * tail;
        }
    }
    let baseline: f64 = mu.iter().sum::<f64>() * t_end;
    -(log_term - baseline - compensator) + penalty
}

/// Deterministic Nelder–Mead (fixed construction; no randomness).
fn nelder_mead(
    objective: impl Fn(&[f64]) -> f64,
    start: &[f64],
    scale: f64,
    iterations: usize,
) -> (Vec<f64>, f64) {
    let n = start.len();
    let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(n + 1);
    simplex.push(start.to_vec());
    for i in 0..n {
        let mut v = start.to_vec();
        v[i] += scale;
        simplex.push(v);
    }
    let mut values: Vec<f64> = simplex.iter().map(|v| objective(v)).collect();

    for _ in 0..iterations {
        // Order ascending by value.
        let mut order: Vec<usize> = (0..=n).collect();
        order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
        let best = order[0];
        let worst = order[n];
        let second_worst = order[n - 1];

        let mut centroid = vec![0.0; n];
        for &i in &order[..n] {
            for (c, x) in centroid.iter_mut().zip(&simplex[i]) {
                *c += x / n as f64;
            }
        }
        let point = |factor: f64| -> Vec<f64> {
            centroid
                .iter()
                .zip(&simplex[worst])
                .map(|(c, w)| c + factor * (c - w))
                .collect()
        };

        let reflected = point(1.0);
        let reflected_value = objective(&reflected);
        if reflected_value < values[best] {
            let expanded = point(2.0);
            let expanded_value = objective(&expanded);
            if expanded_value < reflected_value {
                simplex[worst] = expanded;
                values[worst] = expanded_value;
            } else {
                simplex[worst] = reflected;
                values[worst] = reflected_value;
            }
        } else if reflected_value < values[second_worst] {
            simplex[worst] = reflected;
            values[worst] = reflected_value;
        } else {
            let contracted = point(-0.5);
            let contracted_value = objective(&contracted);
            if contracted_value < values[worst] {
                simplex[worst] = contracted;
                values[worst] = contracted_value;
            } else {
                // Shrink toward the best vertex.
                let best_point = simplex[best].clone();
                for (i, vertex) in simplex.iter_mut().enumerate() {
                    if i != best {
                        for (x, b) in vertex.iter_mut().zip(&best_point) {
                            *x = b + 0.5 * (*x - b);
                        }
                    }
                }
                for (i, vertex) in simplex.iter().enumerate() {
                    values[i] = objective(vertex);
                }
            }
        }
        // Convergence: value spread.
        let spread = values.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b))
            - values.iter().fold(f64::INFINITY, |a, &b| a.min(b));
        if spread < 1e-9 {
            break;
        }
    }
    let mut best_index = 0;
    for (i, &v) in values.iter().enumerate() {
        if v < values[best_index] {
            best_index = i;
        }
    }
    (simplex[best_index].clone(), values[best_index])
}

/// Fit an M-dim exponential-kernel Hawkes (shared β) by MLE.
pub fn fit_exp_mle(events: &Events, t_end: f64) -> Result<FitResult, String> {
    let dims = events.len();
    if dims == 0 || t_end <= 0.0 {
        return Err("no dimensions or empty span".into());
    }
    let n_events: usize = events.iter().map(Vec::len).sum();
    if n_events < 10 * dims {
        return Err(format!("too few events ({n_events}) to fit {dims} dims"));
    }
    let (stream, min_spacing) = merged(events);
    // β can't be estimated below the finest event spacing: a kernel that
    // decays within one spacing is a delta and unidentifiable. Cap at
    // ~20 e-foldings across the finest gap (kernel fully dead by then).
    let beta_max = 20.0 / min_spacing;
    let objective = |theta: &[f64]| negative_ll(theta, &stream, dims, t_end, beta_max);

    // Fixed multi-start grid over (excitation share, decay scale) — the two
    // directions the likelihood is genuinely multimodal/flat in.
    let mean_rate = n_events as f64 / t_end / dims as f64;
    let mut best: Option<(Vec<f64>, f64)> = None;
    for &(share, beta0) in &[(0.2, 0.5), (0.2, 5.0), (0.6, 1.0), (0.6, 20.0)] {
        let mu0 = (mean_rate * (1.0 - share)).max(1e-6);
        let alpha0 = (share * beta0 / dims as f64).max(1e-6);
        let mut start = vec![libm::log(mu0); dims];
        start.extend(vec![libm::log(alpha0); dims * dims]);
        start.push(libm::log(beta0));
        let (theta, value) = nelder_mead(objective, &start, 0.5, 2500);
        if best.as_ref().is_none_or(|(_, b)| value < *b) {
            best = Some((theta, value));
        }
    }
    let (theta, negative_best) = best.expect("at least one start");

    let mu: Vec<f64> = theta[..dims].iter().map(|&x| libm::exp(x)).collect();
    let alpha: Vec<Vec<f64>> = (0..dims)
        .map(|m| {
            (0..dims)
                .map(|n| libm::exp(theta[dims + m * dims + n]))
                .collect()
        })
        .collect();
    let beta = libm::exp(theta[dims + dims * dims]);
    let branching: Vec<Vec<f64>> = alpha
        .iter()
        .map(|row| row.iter().map(|a| a / beta).collect())
        .collect();
    let radius = spectral_radius(&branching);
    let k_params = dims + dims * dims + 1;
    let log_likelihood = -negative_best;

    // Homogeneous-Poisson reference: per-dim MLE rate n_m/T.
    let poisson_ll: f64 = events
        .iter()
        .map(|ts| {
            let n = ts.len() as f64;
            if n == 0.0 {
                0.0
            } else {
                n * libm::log(n / t_end) - n
            }
        })
        .sum();

    Ok(FitResult {
        mu,
        alpha,
        beta,
        log_likelihood,
        aic: 2.0 * k_params as f64 - 2.0 * log_likelihood,
        branching_matrix: branching,
        spectral_radius: radius,
        n_events,
        t_span: t_end,
        poisson_log_likelihood: poisson_ll,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hawkes::simulate::HawkesProcess;
    use sim_core::rng::{SimRng, StreamId};

    /// The numerical-stability tripwire: simulate with known parameters,
    /// refit, recover within tolerance (seeded → deterministic in CI).
    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn simulate_then_fit_recovers_parameters() {
        let true_mu = vec![0.4, 0.4];
        let true_alpha = vec![vec![0.6, 0.15], vec![0.15, 0.6]];
        let true_beta = 1.5; // branching ρ = 0.5
        let mut process =
            HawkesProcess::new(true_mu.clone(), true_alpha.clone(), true_beta).unwrap();
        let mut rng = SimRng::new(1234).stream(StreamId::FLOW_SOURCE);
        let t_end = 30_000.0;
        let mut events: Events = vec![Vec::new(), Vec::new()];
        while let Some((t, dim)) = process.next_event(&mut rng) {
            if t > t_end {
                break;
            }
            events[dim].push(t);
        }
        let n: usize = events.iter().map(Vec::len).sum();
        assert!(n > 20_000, "simulation produced {n} events");

        let fit = fit_exp_mle(&events, t_end).unwrap();
        assert!(
            fit.log_likelihood > fit.poisson_log_likelihood + 100.0,
            "Hawkes must beat Poisson decisively"
        );
        for (m, &mu_true) in true_mu.iter().enumerate() {
            let rel = (fit.mu[m] - mu_true).abs() / mu_true;
            assert!(
                rel < 0.15,
                "mu[{m}] {:.3} vs {mu_true} (rel {rel:.3})",
                fit.mu[m]
            );
        }
        let rel_beta = (fit.beta - true_beta).abs() / true_beta;
        assert!(rel_beta < 0.2, "beta {:.3} vs {true_beta}", fit.beta);
        let true_rho = 0.5;
        assert!(
            (fit.spectral_radius - true_rho).abs() < 0.08,
            "branching ratio {:.3} vs {true_rho}",
            fit.spectral_radius
        );
    }

    #[test]
    fn refuses_tiny_samples() {
        assert!(fit_exp_mle(&vec![vec![1.0, 2.0]], 10.0).is_err());
    }
}
