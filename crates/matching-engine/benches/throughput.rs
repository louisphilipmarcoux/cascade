//! Criterion throughput benchmarks: requests/second through the full engine
//! (validation + matching + event emission) under three workload shapes,
//! for both book implementations.
//!
//! Numbers quoted in the README come from a pinned local machine (see
//! docs/benchmarks/), never from shared CI runners.

#[path = "workload.rs"]
mod workload;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use matching_engine::{ArrayBook, EngineConfig, MatchingEngine};
use sim_core::{EventRecord, EventSeq, EventSink, SimTime};
use std::hint::black_box;
use workload::{NEAR_TOUCH, SWEEPY, SYM, UNIFORM, WorkloadSpec, generate};

/// Counts events without storing them (keeps the sink out of the profile).
struct CountingSink(u64);

impl EventSink for CountingSink {
    fn emit(&mut self, record: EventRecord) {
        self.0 += 1;
        black_box(record.seq);
    }
}

fn run_workload(c: &mut Criterion, name: &str, spec: &WorkloadSpec) {
    const N: usize = 100_000;
    let requests = generate(spec, 42, N);

    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("btree", |b| {
        b.iter(|| {
            let mut engine = MatchingEngine::new_btree(SYM, EngineConfig::default());
            let mut seq = EventSeq::new(0);
            let mut sink = CountingSink(0);
            for (i, request) in requests.iter().enumerate() {
                let ts = SimTime::from_micros(i as u64);
                engine
                    .process(ts, &mut seq, &mut sink, *request)
                    .expect("no corruption");
            }
            black_box(sink.0)
        });
    });

    // The bounded verification book, for scale: linear scans, tiny capacity —
    // run on a truncated request stream so it cannot overflow.
    let small = &requests[..2_000.min(requests.len())];
    group.throughput(Throughput::Elements(small.len() as u64));
    group.bench_function("array_book_small", |b| {
        b.iter(|| {
            let mut engine =
                MatchingEngine::new(SYM, EngineConfig::default(), ArrayBook::<2048>::new());
            let mut seq = EventSeq::new(0);
            let mut sink = CountingSink(0);
            for (i, request) in small.iter().enumerate() {
                let ts = SimTime::from_micros(i as u64);
                engine
                    .process(ts, &mut seq, &mut sink, *request)
                    .expect("no corruption");
            }
            black_box(sink.0)
        });
    });

    group.finish();
}

fn benches(c: &mut Criterion) {
    run_workload(c, "uniform", &UNIFORM);
    run_workload(c, "near_touch", &NEAR_TOUCH);
    run_workload(c, "sweepy", &SWEEPY);
}

criterion_group!(engine_throughput, benches);
criterion_main!(engine_throughput);
