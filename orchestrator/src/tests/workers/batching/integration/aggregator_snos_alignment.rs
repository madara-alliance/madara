use crate::core::config::StarknetVersion;
use crate::tests::workers::batching::helpers::{assert_snos_aggregator_alignment, BatchingTestSetup};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: SNOS batches cannot exceed aggregator batch range (L2)
///
/// This test validates that:
/// - SNOS batches are constrained by aggregator batch boundaries
/// - When processing reaches the end of an aggregator batch, SNOS batch closes
/// - SNOS batch cannot span across multiple aggregator batches
#[rstest]
#[tokio::test]
async fn test_snos_cannot_exceed_aggregator_range() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Aggregator batch covering blocks 0-9
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        blob_len: 100,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Empty SNOS database
    setup.database.expect_get_latest_snos_batch().returning(|| Ok(None));

    // Capture created SNOS batches
    let snos_batches = Arc::new(Mutex::new(Vec::new()));
    let snos_clone = snos_batches.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let snos_update_clone = snos_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 9 {
                Ok(Some(agg_clone.clone()))
            } else {
                Ok(None) // No aggregator batch for blocks > 9
            }
        });

    setup.database.expect_get_next_snos_batch_id().returning(|| Ok(1));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 0-14 available (extends beyond aggregator range)
    setup.setup_block_mocks(0..15, "0.13.2");

    // Build config with L2 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run SNOS batching
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batches
    let snos = snos_batches.lock().unwrap();
    assert!(!snos.is_empty(), "SNOS batches should be created");

    // Get unique SNOS batches
    let mut unique_snos: Vec<SnosBatch> = Vec::new();
    for batch in snos.iter() {
        if !unique_snos.iter().any(|b| b.index == batch.index) {
            unique_snos.push(batch.clone());
        }
    }

    // Verify all SNOS batches are within aggregator range [0-9]
    for snos_batch in &unique_snos {
        assert!(
            snos_batch.start_block >= aggregator_batch.start_block,
            "SNOS batch cannot start before aggregator batch"
        );
        assert!(
            snos_batch.end_block <= aggregator_batch.end_block,
            "SNOS batch cannot extend beyond aggregator batch (SNOS end: {}, Agg end: {})",
            snos_batch.end_block,
            aggregator_batch.end_block
        );
        assert_eq!(
            snos_batch.aggregator_batch_index,
            Some(1),
            "SNOS batch should reference aggregator batch 1"
        );
    }

    // Verify alignment
    assert_snos_aggregator_alignment(&unique_snos, &[aggregator_batch]);

    Ok(())
}

/// Test: Aggregator batch closure does not force SNOS batch closure
///
/// This test validates that:
/// - When an aggregator batch closes, open SNOS batches can remain open
/// - SNOS batches close based on their own closure conditions
/// - SNOS batches can continue within the aggregator batch range
#[rstest]
#[tokio::test]
async fn test_aggregator_closure_independent_of_snos() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Closed aggregator batch covering blocks 0-19
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 19,
        num_blocks: 20,
        blob_len: 200,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Closed, // Aggregator is closed
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Existing open SNOS batch covering blocks 0-9
    let existing_snos = SnosBatch {
        index: 1,
        aggregator_batch_index: Some(1),
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        builtin_weights: crate::tests::utils::default_test_bouncer_weights(),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Open, // SNOS is open
        ..Default::default()
    };

    setup.setup_snos_db_expectations(Some(existing_snos.clone()));

    // Capture updated SNOS batches
    let snos_batches = Arc::new(Mutex::new(Vec::new()));
    let snos_update_clone = snos_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 19 {
                Ok(Some(agg_clone.clone()))
            } else {
                Ok(None)
            }
        });

    setup.database.expect_get_next_snos_batch_id().returning(|| Ok(2));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: More blocks available (10-14)
    setup.setup_block_mocks(10..15, "0.13.2");

    // Build config with L2 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run SNOS batching
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: SNOS batch should continue despite aggregator being closed
    let snos = snos_batches.lock().unwrap();

    if !snos.is_empty() {
        // Get final state of SNOS batch 1
        let final_snos = snos.last().unwrap();

        // Should have extended beyond block 9
        assert!(
            final_snos.end_block > 9,
            "SNOS batch should extend beyond initial range (end: {})",
            final_snos.end_block
        );

        // Should still be within aggregator range
        assert!(
            final_snos.end_block <= 19,
            "SNOS batch should stay within aggregator range"
        );

        // Should still reference aggregator batch 1
        assert_eq!(final_snos.aggregator_batch_index, Some(1));
    }

    Ok(())
}

/// Test: Multiple SNOS batches can exist per aggregator batch
///
/// This test validates that:
/// - Multiple SNOS batches can be created within a single aggregator batch
/// - All SNOS batches share the same aggregator_batch_index
/// - SNOS batches cover the aggregator batch range completely
/// - No gaps or overlaps between SNOS batches
#[rstest]
#[tokio::test]
async fn test_multiple_snos_batches_per_aggregator() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Large aggregator batch covering blocks 0-29
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 29,
        num_blocks: 30,
        blob_len: 300,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Empty SNOS database
    setup.database.expect_get_latest_snos_batch().returning(|| Ok(None));

    // Capture created SNOS batches
    let snos_batches = Arc::new(Mutex::new(Vec::new()));
    let snos_clone = snos_batches.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let snos_update_clone = snos_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 29 {
                Ok(Some(agg_clone.clone()))
            } else {
                Ok(None)
            }
        });

    // Track next SNOS batch ID
    let next_id = Arc::new(Mutex::new(1u64));
    setup.database.expect_get_next_snos_batch_id().returning(move || {
        let id = *next_id.lock().unwrap();
        *next_id.lock().unwrap() += 1;
        Ok(id)
    });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: All 30 blocks available
    setup.setup_block_mocks(0..30, "0.13.2");

    // Build config with L2 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run SNOS batching (may create multiple batches based on config)
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batches
    let snos = snos_batches.lock().unwrap();
    assert!(!snos.is_empty(), "SNOS batches should be created");

    // Get unique SNOS batches
    let mut unique_snos: Vec<SnosBatch> = Vec::new();
    for batch in snos.iter() {
        if !unique_snos.iter().any(|b| b.index == batch.index) {
            unique_snos.push(batch.clone());
        }
    }

    // Verify all SNOS batches reference the same aggregator batch
    for snos_batch in &unique_snos {
        assert_eq!(
            snos_batch.aggregator_batch_index,
            Some(1),
            "All SNOS batches should reference aggregator batch 1"
        );
    }

    // Verify all SNOS batches are within aggregator range
    for snos_batch in &unique_snos {
        assert!(
            snos_batch.end_block <= 29,
            "SNOS batch should be within aggregator range [0-29]"
        );
    }

    // Verify alignment
    assert_snos_aggregator_alignment(&unique_snos, &[aggregator_batch]);

    Ok(())
}

/// Test: SNOS batches across multiple aggregator batches
///
/// This test validates that:
/// - When multiple aggregator batches exist, SNOS batches respect boundaries
/// - Each SNOS batch references the correct aggregator_batch_index
/// - SNOS batches do not span aggregator boundaries
#[rstest]
#[tokio::test]
async fn test_snos_across_multiple_aggregator_batches() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Two aggregator batches
    let agg_batch_1 = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        blob_len: 100,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket_1".to_string(),
        status: AggregatorBatchStatus::Closed,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    let agg_batch_2 = AggregatorBatch {
        index: 2,
        start_block: 10,
        end_block: 19,
        num_blocks: 10,
        blob_len: 100,
        squashed_state_updates_path: "state_update/batch/2.json".to_string(),
        blob_path: "blob/batch/2.bin".to_string(),
        bucket_id: "test_bucket_2".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    // Return latest aggregator batch
    let latest_agg = agg_batch_2.clone();
    setup
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(Some(latest_agg.clone())));

    setup.database.expect_create_aggregator_batch().returning(Ok);
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(|batch, _| Ok(batch.clone()));

    // Setup: Empty SNOS database
    setup.database.expect_get_latest_snos_batch().returning(|| Ok(None));

    // Capture created SNOS batches
    let snos_batches = Arc::new(Mutex::new(Vec::new()));
    let snos_clone = snos_batches.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let snos_update_clone = snos_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg1 = agg_batch_1.clone();
    let agg2 = agg_batch_2.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 9 {
                Ok(Some(agg1.clone()))
            } else if block_num <= 19 {
                Ok(Some(agg2.clone()))
            } else {
                Ok(None)
            }
        });

    let next_id = Arc::new(Mutex::new(1u64));
    setup.database.expect_get_next_snos_batch_id().returning(move || {
        let id = *next_id.lock().unwrap();
        *next_id.lock().unwrap() += 1;
        Ok(id)
    });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 0-19 available
    setup.setup_block_mocks(0..20, "0.13.2");

    // Build config with L2 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run SNOS batching
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batches
    let snos = snos_batches.lock().unwrap();

    if !snos.is_empty() {
        // Get unique SNOS batches
        let mut unique_snos: Vec<SnosBatch> = Vec::new();
        for batch in snos.iter() {
            if !unique_snos.iter().any(|b| b.index == batch.index) {
                unique_snos.push(batch.clone());
            }
        }

        // Verify alignment with both aggregator batches
        let all_agg = vec![agg_batch_1.clone(), agg_batch_2.clone()];
        assert_snos_aggregator_alignment(&unique_snos, &all_agg);

        // Verify SNOS batches don't span aggregator boundaries
        for snos_batch in &unique_snos {
            let agg_index = snos_batch.aggregator_batch_index.expect("Should have aggregator index");

            if agg_index == 1 {
                assert!(
                    snos_batch.end_block <= 9,
                    "SNOS batch in aggregator 1 should be within [0-9]"
                );
            } else if agg_index == 2 {
                assert!(
                    snos_batch.start_block >= 10 && snos_batch.end_block <= 19,
                    "SNOS batch in aggregator 2 should be within [10-19]"
                );
            }
        }
    }

    Ok(())
}
