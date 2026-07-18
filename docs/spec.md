# Quant-Sim: An Integrated Market Simulation, Matching Engine & Research Platform

**A single Rust workspace combining a production-grade limit order book engine, a physically/statistically grounded market simulator, and a rigorously-validated backtesting and research framework.**

---

## 1. Philosophy

Two things separate this project from a typical candidate portfolio piece:

1. **Engineering rigor that's measured, not claimed** — every performance or correctness statement in this project is backed by a benchmark, a test, or a citation.
2. **Intellectual honesty about the frontier** — the goal isn't to look like a finished expert. It's to show you can go genuinely deep into open, current research questions, know precisely where your own knowledge and results run out, and reason clearly about both. That combination — depth plus calibrated humility — is what's actually worth investing years of mentorship in, which is the real thing being evaluated.

Nothing here should be presented as "solving" trading. It's presented as: a well-built system, honestly tested, that explores real open questions in market microstructure and quantitative research.

---

## 2. Architecture

```
quant-sim/
├── matching-engine/    # limit order book core, matching logic
├── market-sim/         # Hawkes-driven & historical-replay order flow
├── backtester/         # strategy execution, PnL/risk, statistical validation
├── strategies/         # Avellaneda-Stoikov MM, OU pairs trading, ML-based
├── research/            # Python/Jupyter — stylized facts, Hurst estimation, ML
├── distributed/         # sharded book + consensus (stretch)
└── cli/                 # scenario runner, report generation
```

Rust for the performance-critical path (matching engine, simulator, execution); Python for the research/statistics layer, matching how Jane Street itself splits OCaml/systems work from Python research work.

---

## 3. Core components

### 3.1 `matching-engine`
- Price-time priority limit order book; limit/market/cancel/modify orders, IOC/FOK as stretch
- Emits an event stream (fills, book deltas) rather than mutating shared state directly — makes everything downstream composable
- **Correctness**: property-based tests (proptest) asserting invariants under randomized order sequences (book stays sorted, volume conserved, no phantom fills)
- **Performance**: criterion.rs benchmarks for orders/sec and p50/p99 latency; a second lock-free implementation (crossbeam/atomics) built only after the safe version is fully proven, with both benchmarked side by side; profiled with `perf`/flamegraph, with flamegraphs and findings committed to the repo

### 3.2 `market-sim`
- **Historical replay** mode using free Level 1/2 data (Binance/Coinbase historical trade+book data)
- **Synthetic generation** driven by a fitted **Hawkes process** (self-exciting point process — order arrivals cluster and decay, matching the real empirical clustering documented in market microstructure literature) rather than a naive Poisson model
- Validated against real data's **stylized facts**: fat-tailed return distributions, volatility clustering, and the known short-term predictive power of order book imbalance — the `research/` notebooks show, with actual statistics, whether the simulator reproduces these or not

### 3.3 `backtester`
- `Strategy` trait executed against the matching engine under configurable latency, slippage, and fee frictions
- Risk and performance tracking: position limits, drawdown, Sharpe/Sortino, and a **deflated Sharpe ratio** correction — this accounts for the fact that testing multiple strategies inflates the best-looking result by chance, which is the single most common way backtests mislead their authors
- Walk-forward (never random) train/test splitting for anything ML-based, with explicit checks for look-ahead bias

### 3.4 `strategies`
- **Avellaneda-Stoikov** optimal market making (2008) — inventory-risk-aware quoting, implemented from the actual paper, benchmarked against a naive "quote around mid" baseline under identical frictions
- **Ornstein-Uhlenbeck pairs trading** — a mean-reverting stochastic process (the same Langevin-equation-derived process used throughout physics) fit to a cointegrated pair, with entry/exit thresholds derived from the fitted process rather than ad hoc rules
- **Almgren-Chriss optimal execution** — a stochastic-control / calculus-of-variations solution to the trade-off between market impact and price risk when unwinding a position; implemented, tested against the matching engine, and compared to naive TWAP execution
- **ML-based short-term direction/imbalance prediction** — gradient-boosted trees on LOB features first (interpretable, strong baseline) before anything deep-learning-shaped; explicitly compared against the classical Hawkes/OU baselines under identical backtest conditions, so the write-up can honestly answer "did the added complexity earn its keep?"

### 3.5 `distributed` (stretch, plays to existing systems background)
- Order book sharded across nodes with a consensus layer for state agreement — an extension of the raft-adjacent work already in the broader portfolio, not distribution bolted on for its own sake
- Chaos testing: injected network delay/partition/reordering, with correctness verified to hold

---

## 4. Engineering quality bar
- Unit + property-based tests on every crate; CI (GitHub Actions) running tests, clippy, and fmt on every push
- Per-crate README explaining design tradeoffs, not just usage
- Reproducibility: pinned historical data snapshots, a `Makefile`/`justfile` to run the full pipeline end to end, and (stretch) a Docker image so anyone can reproduce results exactly
- Clean, incremental commit history that documents the actual research/build process, not one large drop

---

## 5. The research write-up

Structured like a short research report, not a marketing page:
- **Abstract**: what was built and what questions were explored
- **Methodology**: models used, why, and their known assumptions/limitations
- **Results**: benchmarks, backtest statistics, stylized-facts validation — numbers, not adjectives
- **Known Limitations & Future Work** — deliberately included and specific: e.g. "the Hawkes fit doesn't account for cross-excitation between price levels," "the ML model's edge disappears out-of-sample after transaction costs, which itself is an informative negative result," "distributed mode hasn't been tested past 3 nodes." This section is not an afterthought — it's the clearest demonstration of understanding where the real frontier is, and it's the single most credible thing in the whole write-up.
- Published as the top-level README plus, if you want extra reach, a standalone technical blog post

---

## 6. Suggested build order
1. Matching engine core + property tests (everything depends on this being trustworthy)
2. Criterion benchmarks + first profiling pass
3. Historical replay mode in `market-sim`
4. Backtester + trivial baseline strategy, to prove the pipeline end-to-end
5. Avellaneda-Stoikov + naive-MM comparison
6. Hawkes-process fitting + synthetic generation + stylized-facts validation notebook
7. OU pairs trading and Almgren-Chriss execution
8. ML direction-prediction model with walk-forward validation, compared against classical baselines
9. Write-up: methodology, results, and an honest limitations section
10. (Stretch) distributed sharded book + chaos testing
11. (Stretch) lock-free matching engine variant + comparative benchmarks

---

## 7. Data sources
- Binance historical trade/order data (free, no auth for historical downloads)
- Coinbase Exchange historical trades/candles as a cross-validation source

---

## 9. Staying at the actual research frontier

The models in Section 3 (Hawkes, OU, Avellaneda-Stoikov, Almgren-Chriss) are the established foundations — but the field has moved further, and the project is stronger for engaging with where it's moved rather than stopping at the classics.

**What's current right now:**
- **Deep LOB forecasting** has matured well past the original DeepLOB convolutional architecture (Zhang, Zohren, and Roberts). A 2025 study introduces LOBFrame, an open-source pipeline for benchmarking deep learning models on large-scale limit order book data, and finds that a stock's microstructural characteristics — not just model architecture — determine how well deep models forecast mid-price changes across a heterogeneous set of NASDAQ stocks. A separate 2025 transformer-based architecture (LiT) extends this line further using representation learning and transfer learning across the order book.
- **A genuinely useful counter-finding worth engaging with**: a June 2025 study on cryptocurrency order books found that better data preprocessing and input engineering let simple, interpretable models (logistic regression, XGBoost) match or exceed deep architectures like DeepLOB and Conv1D-LSTM on out-of-sample accuracy and latency. This is directly useful for your project's ML section — it's a real, current, citable reason to take the "simple model first, prove complexity earns its keep" discipline seriously rather than treating it as a hedge.
- **Reinforcement learning for optimal execution** is an active area beyond the classical Almgren-Chriss closed-form solution — a 2022 paper frames optimal execution as an RL problem directly, and agent-based market simulators like ABIDES (used in published multi-agent finance research) exist specifically to train and evaluate such RL agents against realistic simulated markets rather than static historical data.
- **Deep order-flow imbalance** research extracts predictive signal from order flow at multiple time horizons rather than a single snapshot — a useful extension once the basic imbalance-prediction baseline is working.

**How to actually stay current (this is a practice, not a one-time reading list):**
- Track new submissions in arXiv's `q-fin.TR` (Trading and Market Microstructure) and `q-fin.ST` (Statistical Finance) categories
- SSRN's market microstructure section for working papers ahead of formal publication
- Keep a `research/papers.md` log in the repo of what you read, what you tried to replicate, and what you'd do differently — this log itself becomes part of the honest "here's where the frontier is and here's my read on it" story from Section 5, and it's something that keeps compounding rather than going stale the moment the project is "finished"

## 10. Broader purpose — not just an application piece

Framing this only as "something to show Jane Street" caps its value and its motivation. It's worth building as something with real standalone use:

- **As an open-source research tool**: a correctly-implemented, well-tested limit order book engine with a Hawkes-driven realistic simulator is genuinely useful to students, independent researchers, and other quant-curious engineers who don't have access to expensive proprietary infrastructure — this is a real gap LOBFrame and ABIDES exist to partially fill, and there's room for a Rust-performance alternative
- **As a personal research platform**: something you keep extending as new papers come out, testing new ideas (a new execution algorithm, a new microstructure model) against a trustworthy backtesting harness — useful for exploring your own strategies well beyond any single job application
- **As a teaching artifact**: the property-based tests, the honest limitations section, and the papers log make it a genuinely good reference for anyone learning market microstructure from a systems-engineering angle, which is an underserved intersection
- **For your existing ventures**: several of the pieces — the Rust performance discipline, the statistical validation rigor, the distributed systems extension — are transferable to other data-heavy or automation-heavy work you already do, so the time invested pays off outside of a single interview outcome

The Jane Street application becomes a natural byproduct of a project worth building for its own sake — which, not coincidentally, is also the version of the project that reads most credibly to the people evaluating it.

## 11. What actually gets an application to the top of the pile (beyond project scope)

Past the point Section 3–10 already reach, more scope stops being the lever. These do more:

- **Finish something completely rather than half-finish something huge.** A recruiter or engineer skimming a GitHub profile can tell instantly whether a repo is real and done or an ambitious README over thin code. Cut scope ruthlessly if needed — matching engine + one rigorously validated strategy + honest write-up, fully finished and polished, beats all ten pieces at 60%.
- **Make it easy to evaluate in under two minutes.** A top-level README with: what it is, a results screenshot/plot, how to run it in one command, and a link to the write-up. Most people deciding whether to look closer make that call almost immediately.
- **A short demo**, even a 2-minute screen recording or a couple of GIFs showing the backtest running and the report it produces, dramatically lowers the effort for someone to actually see it work rather than reading code cold.
- **Get real feedback before applying, from people who'd actually know.** Post the write-up somewhere quant-adjacent engineers hang out (relevant subreddits, a quant finance Discord, Hacker News) and take the critique seriously — folding in a real critique from someone knowledgeable and noting it in the repo is a strong, honest signal in itself, and often surfaces mistakes worth fixing before an interviewer finds them.
- **A referral matters more than almost anything else you can build.** If you know anyone, even distantly, who works in quant trading or at Jane Street specifically, a warm intro or a referred application meaningfully changes whether a resume gets real attention versus sitting in a large pile. Worth spending real effort on before or alongside the build.
- **Apply through the right channel and don't let the project delay the application.** Submit the application now if you're ready, and treat the project as something you keep improving and can point to / send an update on — rather than waiting for it to be "finished" before applying at all, since these processes can take time regardless.
- **The cover letter or application note should tell the reader what to look at first** in the project and why, rather than assuming they'll dig through everything — point them straight at the limitations section and the one result you're most proud of defending.

None of this replaces doing strong technical work — it's what makes strong technical work actually get seen and taken seriously by a person skimming a large stack of applications.

## 12. The options/derivatives extension — a unifying research contribution

Jane Street is one of the largest options market makers in the world, so a derivatives arm is directly relevant in a way generic equity/crypto microstructure work isn't. There's also a genuinely elegant piece of current research that lets this connect back to work already in the project rather than being a bolted-on new module.

**The connection**: research (El Euch, Jusselin, Rosenbaum; extended by Horst et al.) has shown that rough volatility — the empirically-observed fact that realized volatility behaves like a process rougher than Brownian motion (Hurst parameter well below 0.5) — emerges naturally from Hawkes process models of market activity. That means the *same* Hawkes-based order flow model already driving `market-sim` provides a theoretical bridge to a rough-volatility options pricing model. Building both isn't two unrelated features — it's one coherent research thread from microstructure up to derivatives, which is a substantially stronger project narrative than either piece alone.

**Concretely:**
- Implement the **rough Bergomi model** (Bayer, Friz, Gatheral, 2016) — fractional Brownian motion with H < 0.5 driving volatility — for European option pricing via Monte Carlo, since the model's lack of the Markov property rules out simple closed-form pricing
- Calibrate it to real (even if delayed/free) options market data and show it reproduces the empirically observed volatility skew, particularly the steep short-maturity skew that classical (Black-Scholes, Heston) models can't capture
- Estimate the Hurst parameter from your own Hawkes-driven simulator's implied volatility and compare it to the Hurst parameter estimated from real market data — a direct, checkable test of whether the theoretical Hawkes→rough-vol connection holds up in your own system, not just in the papers
- A basic options market-making strategy on top: quote options using the calibrated model, delta/vega-hedge the resulting position using the underlying's `matching-engine`, and evaluate hedging P&L — this closes the loop from microstructure simulation to derivatives risk management, which is close to the actual shape of Jane Street's business

**Computational rigor this requires (and which is worth doing properly, not approximately):**
- Monte Carlo pricing under rough volatility is expensive because the process isn't Markovian — implement variance reduction (antithetic variates, control variates) rather than brute-forcing more paths; this is classical computational-physics technique applied directly, a natural fit for a physics background
- A Markovian approximation of rough Bergomi (multi-factor approximations exist in the literature) is a legitimate way to make pricing tractable and is itself worth implementing and benchmarking against the exact-but-slow version, with the tradeoff documented

## 13. Formal verification and estimation rigor

Two further additions that are rare even among strong candidate projects, precisely because they're genuinely hard:

- **Formal verification of the matching engine's core invariants** using a Rust model checker (Kani, built on top of bounded model checking) — proving properties like "total volume is conserved across any sequence of matches" hold for *all* possible inputs, not just the ones your property-based tests happened to generate. This is a meaningfully higher bar than testing and is uncommon outside safety-critical software.
- **Bayesian estimation with uncertainty quantification** for the Hawkes and OU model parameters (MCMC via a library, rather than a single point-estimate MLE fit) — reporting credible intervals on parameters like the Hawkes decay rate or OU mean-reversion speed is a more honest and more sophisticated statistical practice than a point estimate, and it directly supports the "know where your results stop being trustworthy" framing from Section 5.

## 14. Updated build order (reflecting the full scope)

1. Matching engine core + property tests + formal verification of core invariants
2. Criterion benchmarks + profiling
3. Historical replay mode + Hawkes-fitting + synthetic generation, validated against stylized facts
4. Backtester + Avellaneda-Stoikov vs. naive-MM baseline
5. OU pairs trading + Almgren-Chriss execution
6. Bayesian re-estimation of Hawkes/OU parameters with credible intervals
7. ML direction-prediction with walk-forward validation vs. classical baselines
8. Rough Bergomi options pricing (Monte Carlo + variance reduction), calibrated to real data
9. Hurst-parameter cross-check between the Hawkes-driven simulator and real market data
10. Options market-making strategy with delta/vega hedging via the matching engine
11. Write-up: full methodology, results, and honest limitations, with the Hawkes→rough-vol thread as the central narrative
12. (Stretch) distributed sharded book + chaos testing
13. (Stretch) lock-free matching engine variant + comparative benchmarks

## 15. Web app / GUI layer — making it a usable product, not just a repo

A recruiter who can click a link and watch the matching engine actually run, in their browser, in real time, will remember the project in a way no amount of README prose achieves. This is also the single highest-leverage way to satisfy the "2-minute evaluation" packaging point from Section 11 — a live demo removes all friction between "heard about it" and "saw it work."

### Two audiences, two surfaces
- **The 2-minute visitor** (recruiter, engineer skimming a link): needs a live, in-browser, no-setup demo that shows the system actually working
- **The engaged user** (someone who wants to configure and run real backtests): needs a real app with persistence, configuration, and proper results

### Architecture
- **Core Rust crates stay the source of truth** — `matching-engine` and `backtester` are not reimplemented for the web; they're exposed two ways:
  1. **WASM build of `matching-engine`** compiled to run directly in-browser for the live demo page — visitors watch real matched orders and a real depth chart update client-side, with zero backend cost and zero setup latency. This is also a direct, undeniable proof the engine works, since it's the actual code running, not a mockup.
  2. **A Rust backend API** (axum, with WebSocket support for streaming order book/backtest updates) exposing the full `backtester` and research pipeline for heavier runs (historical replay, Hawkes-fit simulations, options pricing) that are too large for a browser-only demo
- **Frontend**: Next.js + TypeScript, matching your existing full-stack experience. Key screens:
  - **Live order book demo** (WASM-powered) — synthetic or replayed order flow hitting the real matching engine, rendered as a live depth chart
  - **Backtest configurator + results dashboard** — pick a strategy, set parameters, run against historical or synthetic data, see equity curve, drawdown, Sharpe/deflated-Sharpe, trade log
  - **Options/volatility explorer** — the calibrated rough Bergomi implied-vol surface as an interactive 3D or heatmap plot, letting a visitor see the skew the model reproduces
  - **Research dashboard** — the stylized-facts validation and Hurst-parameter cross-check results (Section 12) rendered as real charts, not just notebook output
- **Charting**: recharts or d3 for 2D (equity curves, depth charts), a WebGL-based option (plotly or three.js) for the vol surface

### What "production ready" actually means here (kept honest and scoped)
- Dockerized backend + frontend, deployable with one command (docker-compose)
- CI/CD: automatic deploy on merge to main (Vercel for frontend, Fly.io/Render for the Rust backend — cheap or free tiers are fine for a demo-scale audience)
- Structured logging/tracing (the `tracing` crate) on the backend, and basic uptime/error monitoring — not because this needs to handle real trading load, but because "production ready" should mean "if it breaks, I'll know," not just "it has a database"
- Reasonable rate limiting on the public API so a demo can't be trivially abused
- Tests on the API layer (not just the core crates) so the web layer itself is trustworthy, not just a thin wrapper hiding bugs

### Scope discipline here matters most of all
This is the part of the project most tempting to over-build, and least valuable to over-build. A clean live demo of the matching engine plus a working backtest dashboard is enough — a polished two-screen app beats a half-finished five-screen one, for exactly the reasons in Section 11. Resist adding user accounts, saved strategies, or anything stateful beyond what's needed to run and view a backtest, unless it's genuinely useful to you beyond the demo.

## 16. Making the Next.js app itself the impressive part

Given deep existing Next.js experience, the web layer shouldn't be a generic dashboard bolted onto the Rust work — it should demonstrate the same level of craft as the systems programming. Two dimensions: technical architecture and visual design.

### Technical architecture worth showing off
- **App Router with React Server Components** for the data-heavy screens (backtest results, research dashboard) — fetch and render historical results server-side, stream in with Suspense boundaries so the page is interactive before every chart has loaded
- **Server Actions** for triggering backtest runs and options calibrations, rather than a hand-rolled REST client layer — keeps the Rust API boundary clean while the Next.js side stays idiomatic
- **Streaming + WebSocket integration for the live order book** — the WASM matching engine emits an event stream; render it with incremental updates rather than full re-renders, and handle backpressure sensibly if events arrive faster than the UI can paint
- **Edge runtime where it earns its keep** — e.g. serving the WASM module and initial shell from the edge for fast first paint, while heavier backtest computation stays on the Rust backend
- **Virtualized rendering** (e.g. `@tanstack/react-virtual`) for the order book depth table and trade log — these can have thousands of rows and naive rendering will visibly choke
- **Playwright end-to-end tests** covering the core flows (load the live demo, run a backtest, view results) — testing the product layer, not just the Rust crates, closes the same rigor loop the rest of the project holds itself to
- **Type-safe API boundary** between the Rust backend and Next.js — generate TypeScript types from the Rust API types (e.g. via a shared schema or codegen) so the boundary can't silently drift

### Visual design — earn a distinctive identity, don't default to a template
The subject matter itself should drive the design rather than a generic SaaS-dashboard look. Concretely, before building any screen:
- **Ground it in the actual subject**: this is precision instrumentation for market microstructure — order books, tick-by-tick price action, the mathematical surfaces from the options work. That world (density, precision, motion of flowing data) is where a distinctive visual identity comes from, not a generic dashboard template.
- **Pick a real point of view and commit to it** — for example, a dense, information-forward aesthetic closer to a professional trading terminal than a consumer dashboard, with a deliberate type system (a precise, technical display face for numbers/data, paired with a clean body face) and a signature moment — most naturally, the live order book itself, treated as the hero rather than tucked into a corner
- **One visual signature worth remembering** — the order book depth visualization or the volatility surface are the natural candidates; spend the design effort making one of these genuinely striking rather than spreading polish evenly and thinly across five generic screens
- **Motion used deliberately, not decoratively** — the live order book updating in real time is inherently kinetic and doesn't need extra animation; reserve any additional motion for a single well-chosen moment (e.g. the backtest results resolving) rather than scattering micro-animations everywhere
- **Hold the same quality floor as the rest of the project**: responsive down to mobile, visible keyboard focus, reduced-motion respected — the craft argument is stronger if the UI is held to the same rigor as the Rust code, not treated as a lower-stakes layer
