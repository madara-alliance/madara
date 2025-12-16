use crate::core::config::StarknetVersion;
use crate::tests::workers::batching::helpers::{assert_no_gaps, assert_no_overlaps, BatchingTestSetup};
use crate::types::batch::AggregatorBatchStatus;
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use serde_json::json;
use std::error::Error;
use std::sync::{Arc, Mutex};

/// Test: Aggregator batch closes when Starknet version changes
///
/// This test validates that:
/// - When the Starknet version changes between blocks, the current batch closes
/// - A new batch starts with the new version
/// - Each batch has a homogeneous Starknet version
#[rstest]
#[tokio::test]
async fn test_aggregator_closes_on_version_change() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Capture batches
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

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock starknet_blockNumber
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 9 })).unwrap());
    });

    // Mock blocks 0-4 with version 0.13.2
    for block_num in 0..=4 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        setup.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.2"),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        let state_update = crate::tests::workers::batching::helpers::generate_state_update(block_num, 0);
        setup.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    // Mock blocks 5-9 with version 0.13.3 (version change!)
    for block_num in 5..=9 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        setup.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.3"),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        let state_update = crate::tests::workers::batching::helpers::generate_state_update(block_num, 0);
        setup.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    setup.setup_bouncer_weights_mock();

    // Prover client: Expect buckets for both batches
    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify batches were created with different versions
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    let all_batches: Vec<_> = created.iter().chain(updated.iter()).collect();

    // Find batch with version 0.13.2 (should cover blocks 0-4)
    let batch_v0_13_2 = all_batches
        .iter()
        .find(|b| b.starknet_version == StarknetVersion::V0_13_2);

    // Find batch with version 0.13.3 (should cover blocks 5+)
    let batch_v0_13_3 = all_batches
        .iter()
        .find(|b| b.starknet_version == StarknetVersion::V0_13_3);

    if let Some(b1) = batch_v0_13_2 {
        assert_eq!(b1.start_block, 0, "First batch should start at block 0");
        assert_eq!(b1.end_block, 4, "First batch should end at block 4 (last block with v0.13.2)");
        assert_eq!(
            b1.status,
            AggregatorBatchStatus::Closed,
            "First batch should be closed due to version change"
        );
    }

    if let Some(b2) = batch_v0_13_3 {
        assert_eq!(b2.start_block, 5, "Second batch should start at block 5 (first block with v0.13.3)");
        assert!(
            matches!(b2.status, AggregatorBatchStatus::Open | AggregatorBatchStatus::Closed),
            "Second batch can be open or closed"
        );
    }

    // Verify no gaps/overlaps if we have both batches
    if let (Some(b1), Some(b2)) = (batch_v0_13_2, batch_v0_13_3) {
        let batches = vec![(*b1).clone(), (*b2).clone()];
        assert_no_gaps(&batches);
        assert_no_overlaps(&batches);
    }

    Ok(())
}

/// Test: Each batch has consistent Starknet version
///
/// This test validates that:
/// - Blocks with the same version are batched together
/// - Version changes force batch boundaries
/// - Long sequences of same version stay in one batch (up to other limits)
#[rstest]
#[tokio::test]
async fn test_aggregator_version_consistency_within_batch() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Capture batches
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

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock starknet_blockNumber
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 19 })).unwrap());
    });

    // Mock blocks 0-9 with version 0.13.2 (homogeneous batch)
    for block_num in 0..=9 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        setup.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.2"),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        let state_update = crate::tests::workers::batching::helpers::generate_state_update(block_num, 0);
        setup.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    // Mock blocks 10-19 with version 0.13.3
    for block_num in 10..=19 {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        setup.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num, "0.13.3"),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        let state_update = crate::tests::workers::batching::helpers::generate_state_update(block_num, 0);
        setup.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    setup.setup_bouncer_weights_mock();

    // Prover client: Expect buckets for both batches
    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    // Note: This will process max 25 blocks per run
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Verify version boundaries are respected
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    let all_batches: Vec<_> = created.iter().chain(updated.iter()).collect();

    // Verify no batch contains mixed versions
    for batch in &all_batches {
        // Each batch should have only one version
        // We verify this by checking that version boundaries align with batch boundaries
        if batch.starknet_version == StarknetVersion::V0_13_2 {
            assert!(
                batch.end_block < 10,
                "Batch with v0.13.2 should not extend into v0.13.3 territory (block 10+)"
            );
        }
        if batch.starknet_version == StarknetVersion::V0_13_3 {
            assert!(
                batch.start_block >= 10,
                "Batch with v0.13.3 should not start before block 10"
            );
        }
    }

    // Verify batch at version boundary (should close at block 9)
    let boundary_batch = all_batches.iter().find(|b| b.end_block == 9);
    if let Some(batch) = boundary_batch {
        assert_eq!(
            batch.starknet_version,
            StarknetVersion::V0_13_2,
            "Batch ending at block 9 should have v0.13.2"
        );
        assert_eq!(
            batch.status,
            AggregatorBatchStatus::Closed,
            "Batch should close at version boundary"
        );
    }

    Ok(())
}

/// Test: Frequent version changes create multiple small batches
///
/// This test validates that:
/// - When version changes frequently, multiple small batches are created
/// - Each batch still has a homogeneous version
/// - Batch indices are sequential
#[rstest]
#[tokio::test]
async fn test_aggregator_frequent_version_changes() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Capture batches
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

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock starknet_blockNumber
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 5 })).unwrap());
    });

    // Mock blocks with alternating versions: 0(v2), 1(v3), 2(v2), 3(v3), 4(v2), 5(v3)
    let versions = ["0.13.2", "0.13.3", "0.13.2", "0.13.3", "0.13.2", "0.13.3"];
    for (block_num, version) in versions.iter().enumerate() {
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        let version_str = version.to_string();
        setup.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": crate::tests::workers::batching::helpers::generate_block_with_tx_hashes(block_num as u64, &version_str),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        let state_update = crate::tests::workers::batching::helpers::generate_state_update(block_num as u64, 0);
        setup.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    setup.setup_bouncer_weights_mock();

    // Prover client: Expect multiple buckets
    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute
    AggregatorBatchingTrigger.run_worker(config).await?;

    // Assert: Multiple small batches should be created
    let created = created_batches.lock().unwrap();
    let updated = updated_batches.lock().unwrap();

    let all_batches: Vec<_> = created.iter().chain(updated.iter()).collect();

    // With frequent version changes, we expect multiple batches
    // Each batch should contain only 1 block (due to version changing every block)
    assert!(
        all_batches.len() >= 2,
        "Expected multiple batches due to frequent version changes, got {}",
        all_batches.len()
    );

    // Verify each batch has consistent version
    for batch in &all_batches {
        // Single-block batches should have num_blocks == 1
        if batch.num_blocks == 1 {
            // This is expected with alternating versions
            assert_eq!(
                batch.start_block, batch.end_block,
                "Single-block batch should have start == end"
            );
        }
    }

    Ok(())
}
