//! Per-operation latency soak with full-distribution reporting.
//!
//! Criterion's model centers on means; matching engines are judged on tails.
//! This harness (`cargo bench --bench latency`) times every single request
//! with a monotonic clock, records nanoseconds into an HDR histogram, and
//! prints p50/p90/p99/p99.9/p99.99/max plus throughput — the numbers the
//! README quotes (from pinned local hardware, with the machine noted).

#[path = "workload.rs"]
mod workload;

use hdrhistogram::Histogram;
use matching_engine::{EngineConfig, MatchingEngine};
use sim_core::{EventRecord, EventSeq, EventSink, SimTime};
use std::hint::black_box;
use workload::{NEAR_TOUCH, SWEEPY, SYM, UNIFORM, WorkloadSpec, generate};

struct CountingSink(u64);

impl EventSink for CountingSink {
    fn emit(&mut self, record: EventRecord) {
        self.0 += 1;
        black_box(record.seq);
    }
}

fn soak(name: &str, spec: &WorkloadSpec) {
    const WARMUP: usize = 200_000;
    const MEASURED: usize = 1_000_000;

    let requests = generate(spec, 42, WARMUP + MEASURED);
    let mut engine = MatchingEngine::new_btree(SYM, EngineConfig::default());
    let mut seq = EventSeq::new(0);
    let mut sink = CountingSink(0);
    let mut histogram: Histogram<u64> =
        Histogram::new_with_bounds(1, 10_000_000, 3).expect("histogram bounds");

    for (i, request) in requests.iter().enumerate().take(WARMUP) {
        let ts = SimTime::from_micros(i as u64);
        engine
            .process(ts, &mut seq, &mut sink, *request)
            .expect("no corruption");
    }

    // Wall clock is banned in simulation paths; measuring real latency is the
    // one legitimate use, so the ban is overridden exactly here.
    #[allow(clippy::disallowed_methods)]
    let total_start = std::time::Instant::now();
    for (i, request) in requests.iter().enumerate().skip(WARMUP) {
        let ts = SimTime::from_micros(i as u64);
        #[allow(clippy::disallowed_methods)]
        let start = std::time::Instant::now();
        engine
            .process(ts, &mut seq, &mut sink, *request)
            .expect("no corruption");
        let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        histogram.saturating_record(nanos.max(1));
    }
    let total = total_start.elapsed();

    let per_sec = f64::from(u32::try_from(MEASURED).expect("fits")) / total.as_secs_f64();
    println!("\n== {name} ==");
    println!(
        "requests: {MEASURED} in {:.3}s  ({:.0} req/s)   events emitted: {}",
        total.as_secs_f64(),
        per_sec,
        sink.0
    );
    for (label, q) in [
        ("p50", 50.0),
        ("p90", 90.0),
        ("p99", 99.0),
        ("p99.9", 99.9),
        ("p99.99", 99.99),
    ] {
        println!("  {label:>7}: {:>8} ns", histogram.value_at_percentile(q));
    }
    println!("  {:>7}: {:>8} ns", "max", histogram.max());
}

fn main() {
    // `cargo bench -- --test` (CI smoke) must not run the full soak.
    if std::env::args().any(|a| a == "--test") {
        let requests = generate(&UNIFORM, 7, 1_000);
        let mut engine = MatchingEngine::new_btree(SYM, EngineConfig::default());
        let mut seq = EventSeq::new(0);
        let mut sink = CountingSink(0);
        for (i, request) in requests.iter().enumerate() {
            engine
                .process(
                    SimTime::from_micros(i as u64),
                    &mut seq,
                    &mut sink,
                    *request,
                )
                .expect("no corruption");
        }
        println!("latency soak smoke ok ({} events)", sink.0);
        return;
    }
    soak("uniform", &UNIFORM);
    soak("near_touch", &NEAR_TOUCH);
    soak("sweepy", &SWEEPY);
}
