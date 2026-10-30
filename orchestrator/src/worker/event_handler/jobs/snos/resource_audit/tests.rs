use super::*;
use crate::core::client::database::{DatabaseError, MockDatabaseClient};
use blockifier::bouncer::BouncerWeights;
use rstest::rstest;
use starknet_api::execution_resources::GasAmount;
use std::sync::Arc;

fn fixture(blocks: u64, reservation: u64) -> (SnosBatch, SnosMetadata) {
    let batch = SnosBatch {
        index: 7,
        start_block: 100,
        end_block: 100 + blocks - 1,
        num_blocks: blocks,
        starknet_version: "0.14.2".parse().unwrap(),
        builtin_weights: BouncerWeights { proving_gas: GasAmount(reservation), ..BouncerWeights::empty() },
        ..Default::default()
    };
    let metadata = SnosMetadata {
        snos_batch_index: batch.index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        num_blocks: blocks,
        ..Default::default()
    };
    (batch, metadata)
}

/// A steps-only comparison misses both memory-hole and builtin-heavy deficits. Output is
/// deliberately present because real SNOS PIEs contain it, but it has no price in this model.
#[test]
fn prices_all_resources_with_proving_not_sierra_builtin_costs() {
    let resources = ExecutionResources {
        n_steps: 10,
        n_memory_holes: 3,
        builtin_instance_counter: [(BuiltinName::poseidon, 2), (BuiltinName::output, 500)].into(),
    };
    let (batch, metadata) = fixture(1, 1_000);
    let (actual, reserved) = compare(&batch, &metadata, &resources, None).unwrap();
    assert_eq!(actual, 10 * 100 + 3 * 10 + 2 * 10_000);
    assert!(actual > reserved);
}

/// Reservations are already accumulated across blocks, including the empty-block allowance.
/// Equality is valid; one unit over budget must be detected, even when the budget is zero.
#[rstest]
#[case::below(3, 299, 30_000, "within_reservation")]
#[case::equal(3, 300, 30_000, "within_reservation")]
#[case::exceeded(3, 301, 30_000, "exceeded")]
#[case::empty_allowance(1, 15_000, 1_500_000, "within_reservation")]
#[case::empty_exceeded(1, 15_001, 1_500_000, "exceeded")]
#[case::zero_reservation(1, 1, 0, "exceeded")]
#[tokio::test]
async fn records_batch_outcome_without_failing_job(
    #[case] blocks: u64,
    #[case] steps: usize,
    #[case] reservation: u64,
    #[case] expected: &str,
) {
    let (batch, metadata) = fixture(blocks, reservation);
    let mut database = MockDatabaseClient::new();
    database
        .expect_get_snos_batches()
        .withf(|query| query.indexes == Some(vec![7]) && query.limit == Some(2))
        .once()
        .return_once(move |_| Ok(vec![batch]));
    let resources = ExecutionResources { n_steps: steps, ..Default::default() };
    assert_eq!(record(&database, &metadata, &resources, None).await, expected);
}

#[test]
fn uses_same_constants_override_as_snos() {
    let (batch, metadata) = fixture(1, 150);
    let mut constants = VersionedConstants::get(&"0.14.2".parse().unwrap()).unwrap().clone();
    Arc::make_mut(&mut constants.os_constants).gas_costs.base.step_gas_cost = 200;
    let resources = ExecutionResources { n_steps: 1, ..Default::default() };
    assert_eq!(compare(&batch, &metadata, &resources, None).unwrap(), (100, 150));
    assert_eq!(compare(&batch, &metadata, &resources, Some(&constants)).unwrap(), (200, 150));
}

#[rstest]
#[case::wrong_index(0)]
#[case::wrong_start(1)]
#[case::wrong_end(2)]
#[case::wrong_count(3)]
#[case::inconsistent_range(4)]
fn refuses_to_compare_different_batches(#[case] mismatch: usize) {
    let (mut batch, mut metadata) = fixture(3, 1_000);
    match mismatch {
        0 => batch.index += 1,
        1 => batch.start_block += 1,
        2 => batch.end_block += 1,
        3 => batch.num_blocks += 1,
        4 => {
            batch.num_blocks += 1;
            metadata.num_blocks = batch.num_blocks;
        }
        _ => unreachable!(),
    }
    assert!(compare(&batch, &metadata, &ExecutionResources::default(), None).is_err());
}

#[rstest]
#[case::missing_batch(false)]
#[case::database_error(true)]
#[tokio::test]
async fn missing_audit_evidence_is_not_reported_as_success(#[case] database_error: bool) {
    let (_, metadata) = fixture(1, 100);
    let mut database = MockDatabaseClient::new();
    database.expect_get_snos_batches().once().return_once(move |_| {
        if database_error {
            Err(DatabaseError::KeyNotFound("test".into()))
        } else {
            Ok(vec![])
        }
    });
    assert_eq!(record(&database, &metadata, &ExecutionResources::default(), None).await, "unavailable");
}

#[test]
fn rejects_unpriced_builtin_and_arithmetic_overflow() {
    let costs = BuiltinWeights::default().gas_costs;
    let base = BaseGasCosts { step_gas_cost: 100, memory_hole_gas_cost: 10, ..Default::default() };
    let resources =
        ExecutionResources { builtin_instance_counter: [(BuiltinName::segment_arena, 1)].into(), ..Default::default() };
    assert!(proving_gas(&resources, &base, &costs).is_err());
    let resources = ExecutionResources { n_steps: usize::MAX, ..Default::default() };
    assert!(proving_gas(&resources, &base, &costs).is_err());
    let resources = ExecutionResources { n_steps: 1, n_memory_holes: 1, ..Default::default() };
    let base = BaseGasCosts { step_gas_cost: u64::MAX, memory_hole_gas_cost: 1, ..Default::default() };
    assert!(proving_gas(&resources, &base, &costs).is_err());
}

/// Resource counts from the real 2026-10-30 SNOS block-6 PIE, independently priced by
/// the end-to-end verifier. Keep this vector to catch changes in units or builtin mapping.
#[test]
fn real_snos_pie_matches_independent_proving_cost() {
    let resources = ExecutionResources {
        n_steps: 1_649_904,
        n_memory_holes: 122_742,
        builtin_instance_counter: [
            (BuiltinName::output, 4_817),
            (BuiltinName::range_check, 93_497),
            (BuiltinName::pedersen, 5_469),
            (BuiltinName::bitwise, 1_600),
            (BuiltinName::ec_op, 3),
            (BuiltinName::poseidon, 4_104),
        ]
        .into(),
    };
    let (batch, metadata) = fixture(1, 387_338_402);
    assert_eq!(compare(&batch, &metadata, &resources, None).unwrap(), (235_585_900, 387_338_402));
}
