use crate::tests::workers::batching::helpers::{
    assert_no_gaps, assert_no_overlaps, assert_snos_aggregator_alignment, BatchingTestSetup,
};
use crate::types::batch::{AggregatorBatch, SnosBatch};
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: Full L2 pipeline (Aggregator → SNOS)
///
/// This test validates the complete L2 batching pipeline:
/// - Aggregator batching creates batches from blocks
/// - SNOS batching creates batches within aggregator batches
/// - All boundaries are correct (no gaps, no overlaps)
/// - SNOS batches are properly aligned to aggregator batches
#[rstest]
#[tokio::test]
async fn test_full_pipeline_l2() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));
    setup.database.expect_get_latest_snos_batch().returning(|| Ok(None));

    // Capture created aggregator batches
    let aggregator_batches = Arc::new(Mutex::new(Vec::new()));
    let agg_clone = aggregator_batches.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        agg_clone.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let agg_update_clone = aggregator_batches.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            agg_update_clone.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

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

    // Mock get_aggregator_batch_for_block for SNOS batching
    let agg_for_snos = aggregator_batches.clone();
    setup
        .database
        .expect_get_aggregator_batch_for_block()
        .returning(move |block_num| {
            let batches = agg_for_snos.lock().unwrap();
            Ok(batches
                .iter()
                .find(|b| b.start_block <= block_num && block_num <= b.end_block)
                .cloned())
        });

    setup.database.expect_get_next_snos_batch_id().returning(|| Ok(1));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: 30 blocks available
    setup.setup_block_mocks(0..30, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    // Build config with L2 layer for aggregator dependency
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Run aggregator batching first
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;

    // Execute: Run SNOS batching
    SnosBatchingTrigger.run_worker(config).await?;

    // Assert: Verify aggregator batches
    let agg_batches = aggregator_batches.lock().unwrap();
    assert!(!agg_batches.is_empty(), "Aggregator batches should be created");

    // Get unique aggregator batches (remove duplicates from updates)
    let mut unique_agg: Vec<AggregatorBatch> = Vec::new();
    for batch in agg_batches.iter() {
        if !unique_agg.iter().any(|b| b.index == batch.index) {
            unique_agg.push(batch.clone());
        }
    }

    // Verify aggregator batch boundaries
    assert_no_gaps(&unique_agg);
    assert_no_overlaps(&unique_agg);

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

    // Verify SNOS batches are aligned to aggregator batches
    assert_snos_aggregator_alignment(&unique_snos, &unique_agg);

    // Verify all SNOS batches have aggregator_batch_index set (L2 mode)
    for snos_batch in &unique_snos {
        assert!(
            snos_batch.aggregator_batch_index.is_some(),
            "L2 SNOS batches must have aggregator_batch_index"
        );
    }

    Ok(())
}

/// Test: Full L3 pipeline (SNOS only, no aggregator)
///
/// This test validates the L3 batching pipeline:
/// - SNOS batching works independently without aggregator batches
/// - aggregator_batch_index is None for all SNOS batches
/// - Boundaries are correct
#[rstest]
#[tokio::test]
async fn test_full_pipeline_l3() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty SNOS database, no aggregator batches
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

    setup.database.expect_get_next_snos_batch_id().returning(|| Ok(1));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Setup: 30 blocks available
    setup.setup_block_mocks(0..30, "0.13.2");

    // Build config with L3 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L3).await;

    // Execute: Run SNOS batching only (no aggregator in L3)
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

    // Verify all SNOS batches have aggregator_batch_index = None (L3 mode)
    for snos_batch in &unique_snos {
        assert!(
            snos_batch.aggregator_batch_index.is_none(),
            "L3 SNOS batches must not have aggregator_batch_index"
        );
    }

    // Verify batches cover the expected range
    let first_batch = unique_snos.first().expect("At least one batch should exist");
    assert_eq!(first_batch.start_block, 0, "First batch should start at block 0");

    Ok(())
}

/// Test: Resume after interruption
///
/// This test validates that:
/// - Workers can resume processing after interruption
/// - No blocks are re-processed
/// - State continuity is maintained
/// - Batches pick up where they left off
#[rstest]
#[tokio::test]
async fn test_resume_after_interruption() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Process first 10 blocks, then simulate interruption and process next 10

    // First run: Empty database
    let existing_batch: Arc<Mutex<Option<AggregatorBatch>>> = Arc::new(Mutex::new(None));
    let batch_for_get = existing_batch.clone();
    setup
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(batch_for_get.lock().unwrap().clone()));

    // Capture created/updated batches
    let aggregator_batches = Arc::new(Mutex::new(Vec::new()));
    let agg_clone = aggregator_batches.clone();
    let batch_for_create = existing_batch.clone();
    setup.database.expect_create_aggregator_batch().returning(move |batch| {
        agg_clone.lock().unwrap().push(batch.clone());
        *batch_for_create.lock().unwrap() = Some(batch.clone());
        Ok(batch)
    });

    let agg_update_clone = aggregator_batches.clone();
    let batch_for_update = existing_batch.clone();
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            agg_update_clone.lock().unwrap().push(batch.clone());
            *batch_for_update.lock().unwrap() = Some(batch.clone());
            Ok(batch.clone())
        });

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // First run: blocks 0-9 available
    setup.setup_block_mocks(0..10, "0.13.2");

    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    // Build config with L2 layer
    let config = setup.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: First run (process blocks 0-9)
    AggregatorBatchingTrigger.run_worker(config.clone()).await?;

    let batches_after_first = aggregator_batches.lock().unwrap().clone();
    assert!(!batches_after_first.is_empty(), "Batches should be created in first run");

    // Setup: Second run with additional blocks
    let mut setup2 = BatchingTestSetup::new();

    // Return the batch created in first run
    let last_batch = existing_batch.lock().unwrap().clone();
    setup2
        .database
        .expect_get_latest_aggregator_batch()
        .returning(move || Ok(last_batch.clone()));

    let aggregator_batches2 = Arc::new(Mutex::new(Vec::new()));
    let agg_clone2 = aggregator_batches2.clone();
    setup2.database.expect_create_aggregator_batch().returning(move |batch| {
        agg_clone2.lock().unwrap().push(batch.clone());
        Ok(batch)
    });

    let agg_update_clone2 = aggregator_batches2.clone();
    setup2
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(move |batch, _| {
            agg_update_clone2.lock().unwrap().push(batch.clone());
            Ok(batch.clone())
        });

    setup2.setup_storage_expectations(None);
    setup2.setup_lock_expectations();

    // Second run: blocks 10-19 available
    setup2.setup_block_mocks(10..20, "0.13.2");

    setup2.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    // Build config with L2 layer
    let config2 = setup2.build_with_layer(orchestrator_utils::layer::Layer::L2).await;

    // Execute: Second run (process blocks 10-19)
    AggregatorBatchingTrigger.run_worker(config2).await?;

    let batches_after_second = aggregator_batches2.lock().unwrap().clone();

    // Assert: Verify continuation
    // The worker should either extend the existing batch or create a new one
    // depending on closure conditions

    // Combine all batches
    let all_batches: Vec<AggregatorBatch> = batches_after_first
        .into_iter()
        .chain(batches_after_second)
        .collect();

    // Get unique batches
    let mut unique_batches: Vec<AggregatorBatch> = Vec::new();
    for batch in all_batches {
        if let Some(existing) = unique_batches.iter_mut().find(|b| b.index == batch.index) {
            // Update with latest version
            if batch.num_blocks > existing.num_blocks {
                *existing = batch;
            }
        } else {
            unique_batches.push(batch);
        }
    }

    // Verify no gaps in coverage
    assert_no_gaps(&unique_batches);
    assert_no_overlaps(&unique_batches);

    // Verify blocks 0-19 are covered
    let total_blocks: u64 = unique_batches.iter().map(|b| b.num_blocks).sum();
    assert!(
        total_blocks >= 20,
        "Should have processed at least 20 blocks across both runs"
    );

    Ok(())
}
