//! Differential testing: the Kani-provable `ArrayBook`, the production
//! `BTreeBook`, and the price-band `LadderBook` must be observationally
//! identical.
//!
//! All engines process the same operation sequence; their **entire event
//! streams** must match record-for-record (same fills, same ids, same
//! ordering — byte-identical under hashing) and their snapshots must agree
//! after every operation. This is the bridge that carries Kani's proofs
//! (done on `ArrayBook`) over to both production-grade books.

use matching_engine::testkit::Harness;
use matching_engine::{ArrayBook, Book, EngineConfig, LadderBook, MatchingEngine, SelfMatchPolicy};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use sim_core::ops::Domain;
use sim_core::testkit::op_sequence;
use sim_core::{SymbolId, VecSink};

const SYM: SymbolId = SymbolId::new(0);
/// Submissions per sequence ≤ 40, so live orders can never exceed capacity.
const CAP: usize = 64;

fn policy() -> impl Strategy<Value = SelfMatchPolicy> {
    prop_oneof![
        Just(SelfMatchPolicy::Allow),
        Just(SelfMatchPolicy::CancelResting),
        Just(SelfMatchPolicy::CancelAggressor),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn all_books_observationally_identical(
        ops in op_sequence(Domain::PROPTEST, 40),
        smp in policy(),
    ) {
        let config = EngineConfig { self_match: smp, ..EngineConfig::default() };
        let mut btree = Harness::new(MatchingEngine::new_btree(SYM, config));
        let mut array = Harness::new(MatchingEngine::new(
            SYM,
            config,
            ArrayBook::<CAP>::new(),
        ));
        let mut ladder = Harness::new(MatchingEngine::new(SYM, config, LadderBook::new()));

        for op in &ops {
            let mut btree_sink = VecSink::new();
            let mut array_sink = VecSink::new();
            let mut ladder_sink = VecSink::new();
            btree
                .apply(op, &mut btree_sink)
                .map_err(|e| TestCaseError::fail(format!("btree corruption: {e}")))?;
            array
                .apply(op, &mut array_sink)
                .map_err(|e| TestCaseError::fail(format!("array corruption: {e}")))?;
            ladder
                .apply(op, &mut ladder_sink)
                .map_err(|e| TestCaseError::fail(format!("ladder corruption: {e}")))?;
            prop_assert_eq!(
                &btree_sink.records,
                &array_sink.records,
                "btree/array event streams diverged on {:?}",
                op
            );
            prop_assert_eq!(
                &btree_sink.records,
                &ladder_sink.records,
                "btree/ladder event streams diverged on {:?}",
                op
            );
            prop_assert_eq!(
                btree.engine.book().snapshot(),
                array.engine.book().snapshot(),
                "btree/array snapshots diverged"
            );
            prop_assert_eq!(
                btree.engine.book().snapshot(),
                ladder.engine.book().snapshot(),
                "btree/ladder snapshots diverged"
            );
            array
                .engine
                .book()
                .check_invariants()
                .map_err(|e| TestCaseError::fail(format!("array invariants: {e}")))?;
            ladder
                .engine
                .book()
                .check_invariants()
                .map_err(|e| TestCaseError::fail(format!("ladder invariants: {e}")))?;
        }
    }
}
