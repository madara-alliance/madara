use crate::core::config::StarknetVersion;
use crate::tests::workers::batching::helpers::{
    assert_aggregator_batch_properties, assert_no_gaps, assert_no_overlaps, BatchingTestSetup,
};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: Empty database creates first aggregator batch
///
/// This test validates the happy path scenario where:
/// - The database has no existing aggregator batches
/// - Blocks 0-9 are available from the RPC
/// - The worker creates the first batch covering these blocks
#[rstest]
#[tokio::test]
async fn test_aggregator_empty_db_creates_first_batch() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database (no existing batches)
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Capture the created/updated batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = created_batches.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        batches_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_batches = Arc::new(Mutex::new(Vec::new()));
    let updated_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            updated_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 0-9 available with version 0.13.2
    let start_block = 0;
    let end_block = 9;
    setup.setup_block_mocks(start_block..end_block + 1, "0.13.2");

    // Setup: Prover client should create a bucket for the new batch
    // Note: May be called multiple times due to batch updates
    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket_id".to_string()));

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify batch was created with correct properties
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();
    let all_batches: Vec<_> = created.iter().chain(updated.iter()).collect();

    assert!(!all_batches.is_empty(), "Expected at least one batch to be created or updated");

    // Find the batch with index 1
    let batch = all_batches
        .iter()
        .find(|b| b.index == 1)
        .expect("Expected to find batch with index 1");

    // Assert: Verify basic batch properties
    assert_eq!(batch.index, 1, "Batch index should be 1");
    assert_eq!(batch.start_block, start_block, "Batch should start at block {}", start_block);
    assert!(
        batch.end_block >= start_block && batch.end_block <= end_block,
        "Batch end_block should be between {} and {}, got {}",
        start_block,
        end_block,
        batch.end_block
    );
    // Note: Batch may be Open or Closed depending on whether all blocks were processed
    assert!(
        matches!(batch.status, AggregatorBatchStatus::Open | AggregatorBatchStatus::Closed),
        "Batch status should be Open or Closed, got {:?}",
        batch.status
    );
    assert_eq!(batch.starknet_version, StarknetVersion::V0_13_2, "Batch version should be V0_13_2");

    // Assert: Verify bucket_id was set
    assert_eq!(batch.bucket_id, "test_bucket_id", "Bucket ID should match prover client response");

    // Assert: Verify paths are set correctly
    assert!(!batch.squashed_state_updates_path.is_empty(), "State updates path should be set");
    assert!(!batch.blob_path.is_empty(), "Blob path should be set");

    Ok(())
}

/// Test: Continues existing open batch
///
/// This test validates that when an open batch already exists:
/// - The worker appends new blocks to the existing batch
/// - No new batch is created
/// - The batch properties are updated correctly
#[rstest]
#[tokio::test]
async fn test_aggregator_continues_existing_open_batch() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Existing open batch covering blocks 0-4
    let existing_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 4,
        num_blocks: 5,
        blob_len: 0,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "existing_bucket_id".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(existing_batch.clone()));

    // Storage should return the existing batch's state update for merging
    let state_update = serde_json::from_value(crate::tests::workers::batching::helpers::generate_state_update(0, 0))?;
    setup.setup_storage_expectations(Some(state_update));

    setup.setup_lock_expectations();

    // Setup: Blocks 5-14 available (continuing from existing batch)
    let end_block = 14;
    setup.setup_block_mocks(5..end_block + 1, "0.13.2");

    // Capture the updated batch
    let updated_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            batches_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Prover client should NOT create a new bucket (reusing existing)
    setup.prover.expect_submit_task().times(0);

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify batch was updated, not recreated
    let batches = updated_batches.lock().unwrap();
    assert!(!batches.is_empty(), "Expected at least one batch update");

    // Get the last updated state (final state after all updates)
    let final_batch = batches.last().unwrap();

    // Assert: Batch should have same index but extended end_block
    assert_eq!(final_batch.index, 1, "Batch index should remain 1");
    assert_eq!(final_batch.start_block, 0, "Start block should remain 0");
    assert!(
        final_batch.end_block >= 14,
        "End block should be extended to at least 14, got {}",
        final_batch.end_block
    );
    assert_eq!(final_batch.status, AggregatorBatchStatus::Open, "Batch should remain open");
    assert_eq!(final_batch.bucket_id, "existing_bucket_id", "Bucket ID should remain the same");

    Ok(())
}

/// Test: Starts new batch after closed batch
///
/// This test validates that when a closed batch exists:
/// - The worker creates a new batch starting from the next block
/// - The closed batch remains unchanged
/// - A new bucket is created for the new batch
#[rstest]
#[tokio::test]
async fn test_aggregator_starts_new_batch_after_closed() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Existing closed batch covering blocks 0-9
    let existing_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 9,
        num_blocks: 10,
        blob_len: 100,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "old_bucket_id".to_string(),
        status: AggregatorBatchStatus::Closed,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(existing_batch.clone()));
    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 10-19 available (starting new batch)
    let start_block = 10;
    let end_block = 19;
    setup.setup_block_mocks(start_block..end_block + 1, "0.13.2");

    // Prover client should create a NEW bucket for the new batch
    setup.prover.expect_submit_task().times(1).returning(|_| Ok("new_bucket_id".to_string()));

    // Capture both created and updated batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let updated_batches = Arc::new(Mutex::new(Vec::new()));

    let created_clone = created_batches.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        created_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            updated_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify a new batch was created
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    // Should have created a new batch
    assert!(
        !created.is_empty() || updated.iter().any(|b| b.index == 2),
        "Expected a new batch with index 2 to be created"
    );

    // Find the new batch (index 2)
    let new_batch = created
        .iter()
        .chain(updated.iter())
        .find(|b| b.index == 2)
        .expect("Batch 2 should exist");

    assert_aggregator_batch_properties(
        new_batch,
        2,                         // index
        start_block,               // start_block
        end_block,                 // end_block
        AggregatorBatchStatus::Open, // status
        StarknetVersion::V0_13_2,  // version
    );

    // Assert: New bucket was created
    assert_eq!(new_batch.bucket_id, "new_bucket_id", "New bucket ID should be created");

    // Assert: No gaps between batches
    let all_batches = vec![existing_batch, new_batch.clone()];
    assert_no_gaps(&all_batches);
    assert_no_overlaps(&all_batches);

    Ok(())
}
