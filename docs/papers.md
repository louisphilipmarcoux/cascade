# Papers — reading log with replication notes

Status: `queued` → `skimmed` → `read` → `replicating` → `replicated`.
Every entry that reaches `replicating` carries a "what we did differently"
note — the point of this log is engagement, not a bibliography.

## Microstructure & stylized facts

### Cont (2001) — Empirical properties of asset returns: stylized facts and statistical issues
*Quantitative Finance 1(2).* — **replicated**
The canonical checklist: heavy tails, aggregational gaussianity, no linear
autocorrelation, volatility clustering, slow |r| ACF decay. Our
`stylized_facts` battery implements each as a symmetric real-vs-sim statistic.
**Differently:** we run the identical code path on simulator output and pin a
preregistered expected-failure list (roughness, Zumbach) *before* looking at
sim results — Cont's facts as a falsifiable acceptance test, not a survey.

### Bacry, Mastromatteo, Muzy (2015) — Hawkes processes in finance
*Market Microstructure and Liquidity 1(1).* — **replicated**
Survey grounding our Hawkes usage: branching-ratio interpretation, exp-kernel
tractability, near-criticality of real markets. Our fits land ρ(G) ≈ 0.5 on
BTCUSDT trade arrivals (hour scale), consistent with their reported range
once one notes ρ rises with the fitting window's timescale span.
**Differently:** deterministic Nelder-Mead multi-start (bit-reproducible fits)
rather than stochastic optimization.

### Ogata (1981) — On Lewis' simulation method for point processes
*IEEE Trans. Inf. Theory 27(1).* — **replicated**
Thinning simulation. Implemented in Rust (`market-sim/src/hawkes/simulate.rs`)
with the decaying-intensity upper bound; O(M²) excitation state, no history.

### Ogata (1988) — Statistical models for earthquake occurrences
*JASA 83(401).* — **replicated**
Time-rescaling residual analysis: compensator-transformed inter-arrivals are
iid Exp(1) under the true model. Implemented in
`quantsim_research.hawkes.diagnostics`; tests confirm the true model passes
KS and a mis-specified (Poisson) model is rejected on Hawkes data.

### Ozaki (1979) — Maximum likelihood estimation of Hawkes' self-exciting point processes
*Ann. Inst. Statist. Math. 31.* — **replicated**
The O(N) recursive log-likelihood for exponential kernels. Implemented twice
(Rust + Python) with a cross-implementation agreement test on the same real
fixture: log-likelihoods within 1%, branching within 0.05.

### Blanc, Donier, Bouchaud (2017) — Quadratic Hawkes processes for financial prices
*Quantitative Finance 17(2).* — **read**, replication queued (Stage 2)
Linear Hawkes is time-reversal symmetric at second order and cannot produce
the Zumbach effect; the quadratic feedback term can. Our battery measures
Zumbach/TRA now (preregistered exp-kernel failure); QHawkes lands in M10.

### Zumbach (2010) — Volatility conditional on price trends
*Quantitative Finance 10(4).* — **read**
The time-reversal asymmetry statistic itself: past trends predict future
volatility more than vice versa. Implemented as
`stylized_facts.orderflow.zumbach_asymmetry`.

## Volatility & roughness

### Gatheral, Jaisson, Rosenbaum (2018) — Volatility is rough
*Quantitative Finance 18(6).* — **replicating**
The m(q,Δ) smoothness regression estimating H ≈ 0.1 for realized vol.
Implemented as `hurst.gjr_smoothness_hurst` plus DFA and R/S; calibrated on
synthetic fBm (H ∈ {0.3, 0.5, 0.7} within ±0.12). Preregistered: our
exp-kernel sim will NOT be rough (H ≈ 0.5); the Stage-2 power-law kernel
experiment tests whether near-criticality closes the gap.

### Jusselin, Rosenbaum (2020) — No-arbitrage implies power-law market impact and rough volatility
*Mathematical Finance 30(4).* — **read**
The theory making our Stage-2 experiment falsifiable: near-critical Hawkes
order flow with power-law kernels ⇒ rough volatility in the price. Our sim
can test this mechanically because flow → book → price is one causal chain.

### El Euch, Fukasawa, Rosenbaum (2018) — The microstructural foundations of leverage effect and rough volatility
*Finance and Stochastics 22(2).* — **skimmed**
Complementary microstructure→rough-vol route; queued alongside Stage 2.

### Bayer, Friz, Gatheral (2016) — Pricing under rough volatility
*Quantitative Finance 16(6).* — **queued** (Stage 4)
rBergomi. The Stage-4 crate implements the hybrid scheme for its Volterra
process and calibrates to recorded Deribit surfaces.

### Bennedsen, Lunde, Pakkanen (2017) — Hybrid scheme for Brownian semistationary processes
*Finance and Stochastics 21(4).* — **queued** (Stage 4)
The simulation scheme for the rBergomi kernel singularity.

### El Euch, Rosenbaum (2019) — The characteristic function of rough Heston models
*Mathematical Finance 29(1).* — **queued** (Stage 4, optional second lens)

### Abi Jaber, El Euch (2019) — Multifactor approximation of rough volatility models
*SIAM J. Financial Math. 10(2).* — **queued** (Stage 4)
The Markovian sum-of-exponentials lift; our accuracy-vs-speed table subject.

## Strategies & execution

### Avellaneda, Stoikov (2008) — High-frequency trading in a limit order book
*Quantitative Finance 8(3).* — **replicated**
Finite-horizon inventory-aware MM. Implemented with online EWMA σ and an
online k estimator (ln λ(δ) regression on the strategy's own fill history).
**Differently:** measured against a naive fixed-spread MM under identical
seeds/flow/fees — under adversarial Hawkes flow both lose money, but A-S
loses ~15× less with a smaller drawdown. We report that honestly rather than
tuning until the sign flips: the paper's edge is inventory control, and
that is exactly what shows up.

### Almgren, Chriss (2000) — Optimal execution of portfolio transactions
*Journal of Risk 3(2).* — **replicated**
Closed-form risk-averse liquidation trajectory. Implemented in the
numerically stable e^{−κt} form (the sinh form overflows at moderate κT — a
transcription hazard the paper doesn't flag). Compared vs TWAP through the
real book: A-C front-loads and realizes a smaller shortfall at the demo's
parameters. The assumed-linear vs realized-swept-book impact gap is reported.

### Ning, Lin, Jaimungal (2021) — Double deep Q-learning for optimal execution
*Applied Mathematical Finance 28(4).* — **queued** (Stage 5)
The RL-vs-Almgren-Chriss benchmark design our M16 follows.

## Honest statistics

### Bailey, López de Prado (2012) — The Sharpe ratio efficient frontier
*Journal of Risk 15(2).* — **replicated**
PSR: the probability the true Sharpe exceeds a benchmark given skew/kurtosis.
Implemented in Rust (in-tree normal CDF) and independently in Python; the
cross-check test holds them to ≤1e-7.

### Bailey, López de Prado (2014) — The deflated Sharpe ratio
*Journal of Portfolio Management 40(5).* — **replicated**
DSR: PSR against the expected max Sharpe over K trials.
**Differently:** K is *plumbed from the actual study runner* — a single run
reports DSR = null with reason "K=1". Most backtesters let the user type K in;
ours counts the trials it actually ran. This is the design decision we'd
defend hardest.

### López de Prado (2018) — Advances in Financial Machine Learning
*Wiley.* — **read** (purged CV chapters)
Purged/embargoed walk-forward splits for Stage 5's ML harness.

## LOB modeling & ML frontier

### Cont, Kukanov, Stoikov (2014) — The price impact of order book events
*Journal of Financial Econometrics 12(1).* — **read**
Order-flow imbalance as the fundamental price-impact covariate; OFI features
land in the Stage-5 harness.

### Stoikov (2018) — The micro-price
*Quantitative Finance 18(12).* — **read**
Micro-price as the fair-value estimator; Stage-5 feature.

### Zhang, Zohren, Roberts (2019) — DeepLOB
*IEEE Trans. Signal Processing 67(11).* — **queued** (Stage 5)
The CNN-LSTM LOB baseline our model zoo reimplements (small config).

### Briola et al. (2025) — Deep limit order book forecasting: a microstructural guide (LOBFrame)
*Quantitative Finance 25(7).* — **queued** (Stage 5) [verify exact issue at citation time]
Central finding to test on crypto: microstructure characteristics drive
predictability more than architecture.

### Wang (2025) — Simple statistical models beat deep learning on crypto LOB
*arXiv:2506.05764.* — **queued** (Stage 5) [verify]
The honest deep-vs-simple comparison our M15 replicates on our own data.

### TRADES (2025) — diffusion-based LOB simulation
*arXiv:2502.07071.* — **queued** (Stage 5, M17) [verify]

### LOB-Bench (2025) — evaluation of generative LOB models
*arXiv:2502.09172.* — **queued** (Stage 5, M17) [verify]
The distributional metrics for judging generative flow vs Hawkes/agents.

### ABIDES — Byrd et al. (2020) — ABIDES: towards high-fidelity multi-agent market simulation
*ACM SIGSIM-PADS.* — **skimmed** (Stage 3)
The agent-population design M11 absorbs onto the verified DES core.
