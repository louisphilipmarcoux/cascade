//! Statistical estimators shared by strategies (f64 domain).

use serde::{Deserialize, Serialize};

/// Ornstein–Uhlenbeck fit via the exact discrete AR(1) mapping.
///
/// `dX = θ(μ − X)dt + σ dW` observed at spacing Δ is exactly
/// `X_{k+1} = a + b X_k + ε`, `b = e^{−θΔ}`, `a = μ(1−b)`,
/// `Var(ε) = σ²(1−b²)/(2θ)` — so OLS on the lagged regression inverts to
/// `(θ, μ, σ)` with no discretization bias.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OuFit {
    pub theta: f64,
    pub mu: f64,
    pub sigma: f64,
    /// ln 2 / θ, same units as `dt`.
    pub half_life: f64,
    pub dt: f64,
    pub n: usize,
    /// AR(1) slope (diagnostic; must lie in (0, 1) for mean reversion).
    pub b: f64,
}

impl OuFit {
    /// Stationary standard deviation σ/√(2θ) — the z-score denominator.
    #[must_use]
    pub fn stationary_std(&self) -> f64 {
        self.sigma / libm::sqrt(2.0 * self.theta)
    }

    /// z-score of a level under the fitted stationary law.
    #[must_use]
    pub fn z_score(&self, x: f64) -> f64 {
        (x - self.mu) / self.stationary_std()
    }
}

pub fn fit_ou(values: &[f64], dt: f64) -> Result<OuFit, String> {
    let num = values.len();
    if num < 10 {
        return Err(format!("need ≥ 10 observations, got {num}"));
    }
    if dt <= 0.0 {
        return Err("dt must be positive".into());
    }
    let lag = &values[..num - 1];
    let lead = &values[1..];
    let count = lag.len() as f64;
    let mean_lag = lag.iter().sum::<f64>() / count;
    let mean_lead = lead.iter().sum::<f64>() / count;
    let mut cov = 0.0;
    let mut var = 0.0;
    for (xi, yi) in lag.iter().zip(lead) {
        cov += (xi - mean_lag) * (yi - mean_lead);
        var += (xi - mean_lag) * (xi - mean_lag);
    }
    if var <= 0.0 {
        return Err("degenerate series (zero variance)".into());
    }
    let slope = cov / var;
    if slope <= 0.0 || slope >= 1.0 {
        return Err(format!(
            "AR(1) slope b = {slope:.4} outside (0,1): not mean-reverting"
        ));
    }
    let intercept = mean_lead - slope * mean_lag;
    let mut rss = 0.0;
    for (xi, yi) in lag.iter().zip(lead) {
        let resid = yi - (intercept + slope * xi);
        rss += resid * resid;
    }
    let resid_var = rss / (count - 2.0);
    let theta = -libm::log(slope) / dt;
    let mu = intercept / (1.0 - slope);
    let sigma = libm::sqrt(2.0 * theta * resid_var / (1.0 - slope * slope));
    Ok(OuFit {
        theta,
        mu,
        sigma,
        half_life: core::f64::consts::LN_2 / theta,
        dt,
        n: num,
        b: slope,
    })
}

/// Exponentially-weighted volatility of a return series (per √period).
#[derive(Debug, Clone, Copy)]
pub struct EwmaVol {
    lambda: f64,
    variance: f64,
    initialized: bool,
}

impl EwmaVol {
    #[must_use]
    pub fn new(lambda: f64) -> Self {
        Self {
            lambda,
            variance: 0.0,
            initialized: false,
        }
    }

    pub fn update(&mut self, r: f64) {
        if self.initialized {
            self.variance = self.lambda * self.variance + (1.0 - self.lambda) * r * r;
        } else {
            self.variance = r * r;
            self.initialized = true;
        }
    }

    #[must_use]
    pub fn sigma(&self) -> f64 {
        libm::sqrt(self.variance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng as _;
    use sim_core::rng::{SimRng, StreamId};

    /// Simulate the exact AR(1) representation and recover (θ, μ, σ).
    #[test]
    fn ou_fit_recovers_known_parameters() {
        let (theta, mu, sigma, dt) = (2.0, 5.0, 0.8, 0.01);
        let b = libm::exp(-theta * dt);
        let noise = sigma * libm::sqrt((1.0 - b * b) / (2.0 * theta));
        let mut rng = SimRng::new(77).stream(StreamId::FLOW_SOURCE);
        let mut normal = || {
            let u1: f64 = 1.0 - rng.random::<f64>();
            let u2: f64 = rng.random();
            libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(2.0 * core::f64::consts::PI * u2)
        };
        let mut x = mu;
        let series: Vec<f64> = (0..200_000)
            .map(|_| {
                x = mu + b * (x - mu) + noise * normal();
                x
            })
            .collect();
        let fit = fit_ou(&series, dt).unwrap();
        assert!(
            (fit.theta - theta).abs() / theta < 0.10,
            "theta {:.3}",
            fit.theta
        );
        assert!((fit.mu - mu).abs() < 0.05, "mu {:.3}", fit.mu);
        assert!(
            (fit.sigma - sigma).abs() / sigma < 0.05,
            "sigma {:.3}",
            fit.sigma
        );
        assert!((fit.half_life - core::f64::consts::LN_2 / theta).abs() < 0.05);
    }

    #[test]
    fn ou_fit_rejects_non_mean_reverting() {
        let mut rng = SimRng::new(3).stream(StreamId::FLOW_SOURCE);
        let mut noise = || rng.random::<f64>() - 0.5;

        // Explosive (b ≈ 1.01 > 1): not mean-reverting.
        let mut x = 0.1f64;
        let explosive: Vec<f64> = (0..5_000)
            .map(|_| {
                x = 1.01 * x + 0.01 * noise();
                x
            })
            .collect();
        assert!(
            fit_ou(&explosive, 1.0).is_err(),
            "explosive series must be rejected"
        );

        // Anti-persistent (b < 0): oscillatory, not OU mean reversion.
        let mut prev = 0.0f64;
        let anti: Vec<f64> = (0..5_000)
            .map(|_| {
                let next = -0.5 * prev + noise();
                prev = next;
                next
            })
            .collect();
        assert!(
            fit_ou(&anti, 1.0).is_err(),
            "anti-persistent series must be rejected"
        );
    }

    /// A random walk is a *unit-root* boundary case: OLS gives b ≈ 1⁻ (the
    /// Dickey–Fuller downward bias), so the fit doesn't reject it — but it
    /// must report a very long half-life (θ ≈ 0), never spurious fast
    /// reversion. This is the honest behavior, documented as such.
    #[test]
    fn random_walk_reports_near_zero_reversion() {
        let mut rng = SimRng::new(9).stream(StreamId::FLOW_SOURCE);
        let mut x = 0.0f64;
        let walk: Vec<f64> = (0..50_000)
            .map(|_| {
                x += rng.random::<f64>() - 0.5;
                x
            })
            .collect();
        if let Ok(fit) = fit_ou(&walk, 1.0) {
            assert!(
                fit.theta < 0.01,
                "random walk θ must be ~0, got {}",
                fit.theta
            );
            assert!(fit.half_life > 60.0, "random walk half-life must be long");
        }
    }
}
