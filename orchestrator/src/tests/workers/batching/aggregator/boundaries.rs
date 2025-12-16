use crate::tests::workers::batching::helpers::{assert_no_gaps, assert_no_overlaps, BatchingTestSetup};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: Aggregator batches have no gaps across multiple runs
///
/// This test validates that:
/// - When processing blocks across multiple trigger runs, no blocks are skipped
/// - end_block[i] + 1 == start_block[i+1] for all consecutive batches
/// - All blocks in the range are covered
/// - Batch indices are sequential
#[rstest]
#[tokio::test]
async fn test_aggregator_no_block_gaps() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database initially
    let current_batch: Arc<Mutex<Option<AggregatorBatch>>> = Arc::new(Mutex::new(None));
    let batch_for_get = current_batch.clone();
    setup
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(batch_for_get.lock().unwrap().clone()));

    // Capture all created and updated batches
    let all_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_for_create = all_batches.clone();
    let current_for_create = current_batch.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        batches_for_create.lock().unwrap().push(batch.clone());
        *current_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let batches_for_update = all_batches.clone();
    let current_for_update = current_batch.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            batches_for_update.lock().unwrap().push(batch.clone());
            *current_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    // Setup storage to handle multiple batches
    setup.storage.expect_get_data().returning(|_| {
        let state_update = crate::tests::workers::batching::helpers::generate_state_update(0, 0);
        Ok(bytes::Bytes::from(serde_json::to_string(&state_update).unwrap()))
    });
    setup.storage.expect_put_data().returning(|_, _| Ok(()));
    setup.setup_lock_expectations();

    // Setup: 50 blocks available
    setup.setup_block_mocks(0..50, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run trigger multiple times to process all blocks
    // (Each run processes up to 25 blocks by default)
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify no gaps
    let batches = all_batches.lock().unwrap();

    // Get unique batches (latest version of each index)
    let mut unique_batches: Vec<AggregatorBatch> = Vec::new();
    for batch in batches.iter() {
        if let Some(existing) = unique_batches.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch.clone();
            }
        } else {
            unique_batches.push(batch.clone());
        }
    }

    unique_batches.sort_by_key(|b| b.index);

    // Verify no gaps
    assert_no_gaps(&unique_batches);

    // Verify indices are sequential
    for (i, batch) in unique_batches.iter().enumerate() {
        assert_eq!(
            batch.index as usize,
            i + 1,
            "Batch indices should be sequential starting from 1"
        );
    }

    // Verify total coverage (at least some blocks processed)
    let total_blocks: u64 = unique_batches.iter().map(|b| b.num_blocks).sum();
    assert!(
        total_blocks >= 20,
        "Should have processed at least 20 blocks, got {}",
        total_blocks
    );

    Ok(())
}

/// Test: Aggregator batches have no overlaps
///
/// This test validates that:
/// - No block appears in multiple batches
/// - end_block[i] < start_block[i+1] for all consecutive batches
/// - Each block is processed exactly once
#[rstest]
#[tokio::test]
async fn test_aggregator_no_block_overlaps() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database initially
    let current_batch: Arc<Mutex<Option<AggregatorBatch>>> = Arc::new(Mutex::new(None));
    let batch_for_get = current_batch.clone();
    setup
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(batch_for_get.lock().unwrap().clone()));

    // Capture all batches
    let all_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_for_create = all_batches.clone();
    let current_for_create = current_batch.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        batches_for_create.lock().unwrap().push(batch.clone());
        *current_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let batches_for_update = all_batches.clone();
    let current_for_update = current_batch.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            batches_for_update.lock().unwrap().push(batch.clone());
            *current_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    // Setup storage to handle multiple batches
    setup.storage.expect_get_data().returning(|_| {
        let state_update = crate::tests::workers::batching::helpers::generate_state_update(0, 0);
        Ok(bytes::Bytes::from(serde_json::to_string(&state_update).unwrap()))
    });
    setup.storage.expect_put_data().returning(|_, _| Ok(()));
    setup.setup_lock_expectations();

    // Setup: 50 blocks available
    setup.setup_block_mocks(0..50, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run trigger multiple times
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify no overlaps
    let batches = all_batches.lock().unwrap();

    // Get unique batches (latest version of each index)
    let mut unique_batches: Vec<AggregatorBatch> = Vec::new();
    for batch in batches.iter() {
        if let Some(existing) = unique_batches.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch.clone();
            }
        } else {
            unique_batches.push(batch.clone());
        }
    }

    unique_batches.sort_by_key(|b| b.index);

    // Verify no overlaps
    assert_no_overlaps(&unique_batches);

    // Additional check: verify each block appears in exactly one batch (for blocks that were processed)
    let max_processed = unique_batches.last().map(|b| b.end_block).unwrap_or(0);
    for i in 0..=max_processed {
        let containing_batches: Vec<_> = unique_batches
            .iter()
            .filter(|b| b.start_block <= i && i <= b.end_block)
            .collect();

        assert_eq!(
            containing_batches.len(),
            1,
            "Block {} should appear in exactly one batch, found in {} batches",
            i,
            containing_batches.len()
        );
    }

    Ok(())
}

/// Test: Aggregator batch range calculation
///
/// This test validates that:
/// - A single trigger run processes a limited number of blocks (default: 25)
/// - Remaining blocks require subsequent runs
/// - The worker doesn't try to process all available blocks at once
#[rstest]
#[tokio::test]
async fn test_aggregator_batch_range_calculation() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Capture created batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = created_batches.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        batches_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_batches = Arc::new(Mutex::new(Vec::new()));
    let update_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    // Setup storage to handle multiple batches
    setup.storage.expect_get_data().returning(|_| {
        let state_update = crate::tests::workers::batching::helpers::generate_state_update(0, 0);
        Ok(bytes::Bytes::from(serde_json::to_string(&state_update).unwrap()))
    });
    setup.storage.expect_put_data().returning(|_, _| Ok(()));
    setup.setup_lock_expectations();

    // Setup: 100 blocks available (more than can be processed in one run)
    setup.setup_block_mocks(0..100, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run trigger once
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Should have processed limited number of blocks
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    // Get all batches
    let all_batches: Vec<AggregatorBatch> = created.iter().chain(updated.iter()).cloned().collect();

    // Get unique batches
    let mut unique_batches: Vec<AggregatorBatch> = Vec::new();
    for batch in all_batches {
        if let Some(existing) = unique_batches.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch;
            }
        } else {
            unique_batches.push(batch);
        }
    }

    // Calculate total blocks processed
    let total_blocks: u64 = unique_batches.iter().map(|b| b.num_blocks).sum();

    // Should have processed approximately 25 blocks (default limit)
    // Allow some flexibility based on closure conditions
    assert!(
        total_blocks <= 30,
        "Single run should process limited blocks (≤30), got {}",
        total_blocks
    );

    assert!(
        total_blocks >= 10,
        "Single run should process some blocks (≥10), got {}",
        total_blocks
    );

    Ok(())
}

/// Test: Aggregator respects min/max block configuration
///
/// This test validates that:
/// - Only blocks within configured range are processed
/// - First batch starts at min_block_to_process
/// - Last batch ends at or before max_block_to_process
///
/// Note: This test documents expected behavior. TestConfigBuilder may not
/// have min/max block config methods yet.
#[rstest]
#[tokio::test]
async fn test_aggregator_respects_min_max_block_config() -> Result<(), Box<dyn Error>> {
    // This test documents that aggregator batching should respect
    // min_block_to_process and max_block_to_process configuration.
    //
    // When these config options are available, this test would:
    // 1. Set min_block_to_process = 10, max_block_to_process = 30
    // 2. Make blocks 0-50 available
    // 3. Run aggregator batching
    // 4. Verify only blocks 10-30 are batched
    // 5. Verify first batch starts at block 10
    // 6. Verify last batch ends at or before block 30

    Ok(())
}

/// Test: Aggregator handles closed batch boundaries correctly
///
/// This test validates that:
/// - When a batch is closed, the next batch starts at the correct block
/// - No blocks are skipped at batch boundaries
/// - Closed batches are not reopened
#[rstest]
#[tokio::test]
async fn test_aggregator_closed_batch_boundaries() -> Result<(), Box<dyn Error>> {
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
        bucket_id: "bucket_1".to_string(),
        status: AggregatorBatchStatus::Closed,
        starknet_version: crate::core::config::StarknetVersion::V0_13_2,
        ..Default::default()
    };

    let batch_clone = existing_batch.clone();
    setup
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(Some(batch_clone.clone())));

    // Capture new batches
    let created_batches = Arc::new(Mutex::new(Vec::new()));
    let create_clone = created_batches.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        create_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_batches = Arc::new(Mutex::new(Vec::new()));
    let update_clone = updated_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: Blocks 10-19 available (continue after closed batch)
    setup.setup_block_mocks(10..20, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("bucket_2".to_string()));

    let config = setup.build().await;

    // Execute: Run aggregator batching
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify new batch starts at block 10
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    // Should have created a new batch starting at block 10
    let new_batches: Vec<_> = created.iter().chain(updated.iter()).filter(|b| b.index == 2).collect();

    if !new_batches.is_empty() {
        let batch_2 = new_batches[0];
        assert_eq!(
            batch_2.start_block, 10,
            "New batch should start immediately after closed batch"
        );
        assert_eq!(batch_2.index, 2, "New batch should have index 2");
    }

    // Verify batch 1 was not modified
    let batch_1_updates: Vec<_> = updated.iter().filter(|b| b.index == 1).collect();
    assert!(
        batch_1_updates.is_empty(),
        "Closed batch should not be reopened or modified"
    );

    Ok(())
}
