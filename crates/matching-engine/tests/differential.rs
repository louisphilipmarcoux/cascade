//! Differential testing: the Kani-provable `ArrayBook` and the production
//! `BTreeBook` must be observationally identical.
//!
//! Both engines process the same operation sequence; their **entire event
//! streams** must match record-for-record (same fills, same ids, same
//! ordering — byte-identical under hashing) and their snapshots must agree
//! after every operation. This is the bridge that carries Kani's proofs
//! (done on `ArrayBook`) over to the production book.

use matching_engine::testkit::Harness;
use matching_engine::{ArrayBook, Book, EngineConfig, MatchingEngine, SelfMatchPolicy};
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
    fn array_book_equals_btree_book(
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

        for op in &ops {
            let mut btree_sink = VecSink::new();
            let mut array_sink = VecSink::new();
            btree
                .apply(op, &mut btree_sink)
                .map_err(|e| TestCaseError::fail(format!("btree corruption: {e}")))?;
            array
                .apply(op, &mut array_sink)
                .map_err(|e| TestCaseError::fail(format!("array corruption: {e}")))?;
            prop_assert_eq!(
                &btree_sink.records,
                &array_sink.records,
                "event streams diverged on {:?}",
                op
            );
            prop_assert_eq!(
                btree.engine.book().snapshot(),
                array.engine.book().snapshot(),
                "snapshots diverged"
            );
            array
                .engine
                .book()
                .check_invariants()
                .map_err(|e| TestCaseError::fail(format!("array invariants: {e}")))?;
        }
    }
}
