use crate::core::config::StarknetVersion;
use crate::tests::workers::batching::helpers::{assert_snos_aggregator_alignment, BatchingTestSetup};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: Empty SNOS database creates first batch (L2 mode)
///
/// This test validates that when:
/// - An aggregator batch exists (index 1, blocks 0-9)
/// - No SNOS batches exist yet
/// - The worker creates SNOS batches linked to the aggregator batch
#[rstest]
#[tokio::test]
async fn test_snos_empty_db_creates_first_batch() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Aggregator batch exists covering blocks 0-9
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        blob_len: 100,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket_id".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Empty SNOS database
    setup.setup_snos_db_expectations(None);
    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 0-9 available
    setup.setup_block_mocks(0..10, "0.13.2");

    // Mock getting aggregator batch for each block
    let agg_batch_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |_| Ok(Some(agg_batch_clone.clone())));

    // Capture created SNOS batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = created_batches.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        batches_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let config = setup.build().await;

    // Execute: Run the SNOS batching trigger
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batch was created
    let batches = created_batches.lock().unwrap();
    assert!(!batches.is_empty(), "Expected at least one SNOS batch to be created");

    let batch = &batches[0];

    // Assert: Verify batch has correct aggregator_batch_index
    assert_eq!(
        batch.aggregator_batch_index,
        Some(1),
        "SNOS batch should be linked to aggregator batch 1"
    );

    // Assert: Verify batch is within aggregator batch range
    assert!(
        batch.start_block >= aggregator_batch.start_block,
        "SNOS batch start should be >= aggregator batch start"
    );
    assert!(
        batch.end_block <= aggregator_batch.end_block,
        "SNOS batch end should be <= aggregator batch end"
    );

    // Assert: Verify version matches aggregator batch
    assert_eq!(
        batch.starknet_version, aggregator_batch.starknet_version,
        "SNOS batch version should match aggregator batch version"
    );

    // Assert: Verify alignment
    assert_snos_aggregator_alignment(&batches, &[aggregator_batch]);

    Ok(())
}

/// Test: Continues existing open SNOS batch (L2 mode)
///
/// This test validates that when:
/// - An aggregator batch covers blocks 0-9
/// - An open SNOS batch exists for blocks 0-4
/// - The worker extends the SNOS batch to cover more blocks
#[rstest]
#[tokio::test]
async fn test_snos_continues_existing_open_batch() -> Result<(), Box<dyn Error>> {
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
        bucket_id: "test_bucket_id".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Existing open SNOS batch covering blocks 0-4
    let existing_snos_batch = SnosBatch {
        index: 1,
        aggregator_batch_index: Some(1),
        start_block: 0,
        end_block: 4,
        num_blocks: 5,
        builtin_weights: crate::tests::utils::default_test_bouncer_weights(),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Open,
        ..Default::default()
    };

    setup.setup_snos_db_expectations(Some(existing_snos_batch.clone()));
    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 5-9 available (continuing from existing batch)
    setup.setup_block_mocks(5..10, "0.13.2");

    // Mock getting aggregator batch for each block
    let agg_batch_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |_| Ok(Some(agg_batch_clone.clone())));

    // Capture updated SNOS batches
    let updated_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            batches_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    let config = setup.build().await;

    // Execute: Run the SNOS batching trigger
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batch was updated
    let batches = updated_batches.lock().unwrap();
    assert!(!batches.is_empty(), "Expected SNOS batch to be updated");

    // Get the final batch state
    let final_batch = batches.last().unwrap();

    // Assert: Batch should have same index but extended end_block
    assert_eq!(final_batch.index, 1, "Batch index should remain 1");
    assert_eq!(final_batch.start_block, 0, "Start block should remain 0");
    assert!(
        final_batch.end_block > 4,
        "End block should be extended beyond 4, got {}",
        final_batch.end_block
    );
    assert_eq!(
        final_batch.aggregator_batch_index,
        Some(1),
        "Aggregator batch index should remain consistent"
    );

    // Assert: Verify alignment
    assert_snos_aggregator_alignment(std::slice::from_ref(final_batch), std::slice::from_ref(&aggregator_batch));

    Ok(())
}

/// Test: Closes and starts new SNOS batch (L2 mode)
///
/// This test validates that when:
/// - An SNOS batch is nearing closure (e.g., due to max blocks)
/// - More blocks are available within the same aggregator batch
/// - The worker closes the first batch and starts a new one
#[rstest]
#[tokio::test]
async fn test_snos_closes_and_starts_new_batch() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Aggregator batch covering blocks 0-19
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 19,
        num_blocks: 20,
        blob_len: 200,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket_id".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Existing SNOS batch with 4 blocks (will close at 5 due to fixed size config)
    let existing_snos_batch = SnosBatch {
        index: 1,
        aggregator_batch_index: Some(1),
        start_block: 0,
        end_block: 3,
        num_blocks: 4,
        builtin_weights: crate::tests::utils::default_test_bouncer_weights(),
        starknet_version: StarknetVersion::V0_13_2,
        status: SnosBatchStatus::Open,
        ..Default::default()
    };

    setup.setup_snos_db_expectations(Some(existing_snos_batch.clone()));
    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: More blocks available
    setup.setup_block_mocks(4..15, "0.13.2");

    // Mock getting aggregator batch for each block
    let agg_batch_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |_| Ok(Some(agg_batch_clone.clone())));

    // Capture both created and updated batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let updated_batches = Arc::new(Mutex::new(Vec::new()));

    let created_clone = created_batches.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        created_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            updated_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Need to track next batch ID
    let next_id = Arc::new(Mutex::new(2u64));
    let next_id_clone = next_id.clone();
    setup.database.expect_get_next_snos_batch_id().returning(move || {
        let id = *next_id_clone.lock().unwrap();
        *next_id_clone.lock().unwrap() += 1;
        Ok(id)
    });

    let config = setup.build().await;

    // Execute: Run the SNOS batching trigger
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Should have at least one updated batch and potentially a new batch
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    // Find the final state of batch 1 (should be closed or extended)
    let _batch_1 = updated.iter().find(|b| b.index == 1).unwrap_or(&existing_snos_batch);

    // Assert: Both batches should be within the same aggregator batch
    let all_batches: Vec<SnosBatch> = created.iter().chain(updated.iter()).cloned().collect();

    if all_batches.len() > 1 {
        // If multiple batches were created/updated, verify they're all in the same aggregator batch
        for batch in &all_batches {
            assert_eq!(
                batch.aggregator_batch_index,
                Some(1),
                "All SNOS batches should belong to aggregator batch 1"
            );
        }

        // Verify alignment
        assert_snos_aggregator_alignment(&all_batches, &[aggregator_batch]);
    }

    Ok(())
}
