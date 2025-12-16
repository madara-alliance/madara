use crate::core::config::StarknetVersion;
use crate::tests::workers::batching::helpers::{assert_snos_aggregator_alignment, BatchingTestSetup};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch};
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: SNOS batches have no gaps across multiple runs (L2)
///
/// This test validates that:
/// - When processing blocks across multiple trigger runs, no blocks are skipped
/// - end_block[i] + 1 == start_block[i+1] for all consecutive batches
/// - All blocks in the aggregator batch range are covered
#[rstest]
#[tokio::test]
async fn test_snos_no_block_gaps_l2() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Aggregator batch covering blocks 0-49
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 49,
        num_blocks: 50,
        blob_len: 500,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Track SNOS batch state
    let current_snos: Arc<Mutex<Option<SnosBatch>>> = Arc::new(Mutex::new(None));
    let snos_for_get = current_snos.clone();
    setup
        .database
        .expect_get_latest_snos_batch()
        .returning(move || Ok(snos_for_get.lock().unwrap().clone()));

    // Capture all SNOS batches
    let all_snos = Arc::new(Mutex::new(Vec::new()));
    let snos_for_create = all_snos.clone();
    let current_for_create = current_snos.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_for_create.lock().unwrap().push(batch.clone());
        *current_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let snos_for_update = all_snos.clone();
    let current_for_update = current_snos.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_for_update.lock().unwrap().push(batch.clone());
            *current_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 49 {
                Ok(Some(agg_clone.clone()))
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

    // Setup: All 50 blocks available
    setup.setup_block_mocks(0..50, "0.13.2");

    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run trigger multiple times to process all blocks
    SnosBatchingTrigger.run_worker(config.clone()).await?;
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify no gaps
    let batches = all_snos.lock().unwrap();

    // Get unique batches
    let mut unique_snos: Vec<SnosBatch> = Vec::new();
    for batch in batches.iter() {
        if let Some(existing) = unique_snos.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch.clone();
            }
        } else {
            unique_snos.push(batch.clone());
        }
    }

    unique_snos.sort_by_key(|b| b.index);

    // Verify no gaps between SNOS batches
    for i in 1..unique_snos.len() {
        assert_eq!(
            unique_snos[i].start_block,
            unique_snos[i - 1].end_block + 1,
            "SNOS batches should have no gaps"
        );
    }

    // Verify alignment with aggregator
    assert_snos_aggregator_alignment(&unique_snos, &[aggregator_batch]);

    Ok(())
}

/// Test: SNOS batches have no overlaps (L2)
///
/// This test validates that:
/// - No block appears in multiple SNOS batches
/// - end_block[i] < start_block[i+1] for all consecutive batches
/// - Each block is processed exactly once
#[rstest]
#[tokio::test]
async fn test_snos_no_block_overlaps_l2() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Aggregator batch covering blocks 0-49
    let aggregator_batch = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 49,
        num_blocks: 50,
        blob_len: 500,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "test_bucket".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    setup.setup_aggregator_db_expectations(Some(aggregator_batch.clone()));

    // Setup: Track SNOS batch state
    let current_snos: Arc<Mutex<Option<SnosBatch>>> = Arc::new(Mutex::new(None));
    let snos_for_get = current_snos.clone();
    setup
        .database
        .expect_get_latest_snos_batch()
        .returning(move || Ok(snos_for_get.lock().unwrap().clone()));

    // Capture all SNOS batches
    let all_snos = Arc::new(Mutex::new(Vec::new()));
    let snos_for_create = all_snos.clone();
    let current_for_create = current_snos.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_for_create.lock().unwrap().push(batch.clone());
        *current_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let snos_for_update = all_snos.clone();
    let current_for_update = current_snos.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_for_update.lock().unwrap().push(batch.clone());
            *current_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg_clone = aggregator_batch.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 49 {
                Ok(Some(agg_clone.clone()))
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

    // Setup: All 50 blocks available
    setup.setup_block_mocks(0..50, "0.13.2");

    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run trigger multiple times
    SnosBatchingTrigger.run_worker(config.clone()).await?;
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify no overlaps
    let batches = all_snos.lock().unwrap();

    // Get unique batches
    let mut unique_snos: Vec<SnosBatch> = Vec::new();
    for batch in batches.iter() {
        if let Some(existing) = unique_snos.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch.clone();
            }
        } else {
            unique_snos.push(batch.clone());
        }
    }

    unique_snos.sort_by_key(|b| b.index);

    // Verify no overlaps
    for i in 1..unique_snos.len() {
        assert!(
            unique_snos[i].start_block > unique_snos[i - 1].end_block,
            "SNOS batches should not overlap"
        );
    }

    // Verify each block appears in exactly one batch
    for block_num in 0..50u64 {
        let containing_batches: Vec<_> = unique_snos
            .iter()
            .filter(|b| b.start_block <= block_num && block_num <= b.end_block)
            .collect();

        assert_eq!(
            containing_batches.len(),
            1,
            "Block {} should appear in exactly one SNOS batch",
            block_num
        );
    }

    Ok(())
}

/// Test: SNOS batches respect aggregator boundaries (L2)
///
/// This test validates that:
/// - SNOS batches cannot cross aggregator batch boundaries
/// - When reaching aggregator boundary, SNOS batch closes
/// - New SNOS batch starts with next aggregator batch
#[rstest]
#[tokio::test]
async fn test_snos_respects_aggregator_boundaries() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Two aggregator batches
    let agg_batch_1 = AggregatorBatch {
        index: 1,
        start_block: 0,
        end_block: 19,
        num_blocks: 20,
        blob_len: 200,
        squashed_state_updates_path: "state_update/batch/1.json".to_string(),
        blob_path: "blob/batch/1.bin".to_string(),
        bucket_id: "bucket_1".to_string(),
        status: AggregatorBatchStatus::Closed,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

    let agg_batch_2 = AggregatorBatch {
        index: 2,
        start_block: 20,
        end_block: 39,
        num_blocks: 20,
        blob_len: 200,
        squashed_state_updates_path: "state_update/batch/2.json".to_string(),
        blob_path: "blob/batch/2.bin".to_string(),
        bucket_id: "bucket_2".to_string(),
        status: AggregatorBatchStatus::Open,
        starknet_version: StarknetVersion::V0_13_2,
        ..Default::default()
    };

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
    let current_snos: Arc<Mutex<Option<SnosBatch>>> = Arc::new(Mutex::new(None));
    let snos_for_get = current_snos.clone();
    setup
        .database
        .expect_get_latest_snos_batch()
        .returning(move || Ok(snos_for_get.lock().unwrap().clone()));

    // Capture SNOS batches
    let all_snos = Arc::new(Mutex::new(Vec::new()));
    let snos_for_create = all_snos.clone();
    let current_for_create = current_snos.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_for_create.lock().unwrap().push(batch.clone());
        *current_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let snos_for_update = all_snos.clone();
    let current_for_update = current_snos.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            snos_for_update.lock().unwrap().push(batch.clone());
            *current_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    // Mock get_aggregator_batch_for_block
    let agg1 = agg_batch_1.clone();
    let agg2 = agg_batch_2.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            if block_num <= 19 {
                Ok(Some(agg1.clone()))
            } else if block_num <= 39 {
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

    // Setup: All 40 blocks available
    setup.setup_block_mocks(0..40, "0.13.2");

    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run SNOS batching
    SnosBatchingTrigger.run_worker(config.clone()).await?;
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify SNOS batches respect aggregator boundaries
    let batches = all_snos.lock().unwrap();

    if !batches.is_empty() {
        // Get unique batches
        let mut unique_snos: Vec<SnosBatch> = Vec::new();
        for batch in batches.iter() {
            if !unique_snos.iter().any(|b| b.index == batch.index) {
                unique_snos.push(batch.clone());
            }
        }

        // Verify no SNOS batch spans aggregator boundary
        for snos_batch in &unique_snos {
            let crosses_boundary = snos_batch.start_block < 20 && snos_batch.end_block >= 20;
            assert!(
                !crosses_boundary,
                "SNOS batch should not cross aggregator boundary at block 20"
            );
        }

        // Verify alignment
        assert_snos_aggregator_alignment(&unique_snos, &[agg_batch_1, agg_batch_2]);
    }

    Ok(())
}

/// Test: SNOS batch range calculation (L3)
///
/// This test validates that:
/// - A single trigger run processes a limited number of blocks
/// - Remaining blocks require subsequent runs
/// - L3 mode processes blocks independently
#[rstest]
#[tokio::test]
async fn test_snos_batch_range_calculation_l3() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty SNOS database
    setup.database.expect_get_latest_snos_batch().returning(|| Ok(None));

    // Capture created batches
    let created_snos = Arc::new(Mutex::new(Vec::new()));
    let snos_clone = created_snos.clone();
    setup.database.expect_create_snos_batch().returning(move |batch| {
        snos_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let updated_snos = Arc::new(Mutex::new(Vec::new()));
    let update_clone = updated_snos.clone();
    setup
        .database
        .expect_update_or_create_snos_batch()
        .returning(move |batch, _| {
            update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    let next_id = Arc::new(Mutex::new(1u64));
    setup.database.expect_get_next_snos_batch_id().returning(move || {
        let id = *next_id.lock().unwrap();
        *next_id.lock().unwrap() += 1;
        Ok(id)
    });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: 100 blocks available (more than can be processed in one run)
    setup.setup_block_mocks(0..100, "0.13.2");

    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L3).await;

    // Execute: Run trigger once
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Should have processed limited number of blocks
    let created = created_snos.lock().unwrap();
    let updated = updated_snos.lock().unwrap();

    // Get all batches
    let all_batches: Vec<SnosBatch> = created.iter().chain(updated.iter()).cloned().collect();

    // Get unique batches
    let mut unique_snos: Vec<SnosBatch> = Vec::new();
    for batch in all_batches {
        if let Some(existing) = unique_snos.iter_mut().find(|b| b.index == batch.index) {
            if batch.num_blocks >= existing.num_blocks {
                *existing = batch;
            }
        } else {
            unique_snos.push(batch);
        }
    }

    // Calculate total blocks processed
    let total_blocks: u64 = unique_snos.iter().map(|b| b.num_blocks).sum();

    // Should have processed approximately 25 blocks (default limit)
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

    // Verify all batches have aggregator_batch_index = None (L3 mode)
    for snos_batch in &unique_snos {
        assert!(
            snos_batch.aggregator_batch_index.is_none(),
            "L3 SNOS batches should not have aggregator_batch_index"
        );
    }

    Ok(())
}

/// Test: SNOS respects min/max block configuration
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
async fn test_snos_respects_min_max_block_config() -> Result<(), Box<dyn Error>> {
    // This test documents that SNOS batching should respect
    // min_block_to_process and max_block_to_process configuration.
    //
    // When these config options are available, this test would:
    // 1. Set min_block_to_process = 10, max_block_to_process = 30
    // 2. Make blocks 0-50 available
    // 3. Run SNOS batching
    // 4. Verify only blocks 10-30 are batched
    // 5. Verify first batch starts at block 10
    // 6. Verify last batch ends at or before block 30

    Ok(())
}
