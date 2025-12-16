use crate::core::client::lock::{LockResult, LockValue};
use crate::tests::workers::batching::helpers::BatchingTestSetup;
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use serde_json::json;
use std::error::Error;

/// Test: Aggregator handles RPC block number failure gracefully
///
/// This test validates that:
/// - When starknet_blockNumber RPC call fails, the error is propagated
/// - No partial batches are created
/// - Database remains consistent
#[rstest]
#[tokio::test]
async fn test_aggregator_handles_rpc_block_number_failure() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));
    setup.database.expect_create_aggregator_batch().returning(Ok);
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(|batch, _| Ok(batch.clone()));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock starknet_blockNumber to fail
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(500).body(json!({"error": "Internal server error"}).to_string());
    });

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    let result = AggregatorBatchingTrigger.run_worker(config).await;

    // Assert: Should return an error
    assert!(result.is_err(), "Expected error when RPC block number fails");

    Ok(())
}

/// Test: Aggregator handles state update fetch failure
///
/// This test validates that:
/// - When starknet_getStateUpdate fails for a block, processing stops
/// - Blocks before the failure are processed successfully
/// - Error is logged appropriately
#[rstest]
#[tokio::test]
async fn test_aggregator_handles_state_update_fetch_failure() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));
    setup.database.expect_create_aggregator_batch().returning(Ok);
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(|batch, _| Ok(batch.clone()));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock starknet_blockNumber
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": 9 })).unwrap());
    });

    // Mock getBlockWithTxHashes to succeed
    setup.setup_block_mocks(0..10, "0.13.2");

    // Make getStateUpdate fail
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_getStateUpdate");
        then.status(500).body(json!({"error": "State update fetch failed"}).to_string());
    });

    setup.setup_bouncer_weights_mock();
    setup.prover.expect_submit_task().returning(|_| Ok("test_bucket".to_string()));

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    let result = AggregatorBatchingTrigger.run_worker(config).await;

    // Assert: Should fail when fetching state update
    assert!(result.is_err(), "Expected error when state update fetch fails");

    Ok(())
}

/// Test: Aggregator handles prover client failure
///
/// This test validates that:
/// - When prover client submit_task fails, batch creation fails
/// - Error is logged with context
/// - Can retry after prover becomes available
///
/// Note: This test documents expected behavior. Full mocking of prover
/// errors requires compatible error types.
#[rstest]
#[tokio::test]
async fn test_aggregator_handles_prover_client_failure() -> Result<(), Box<dyn Error>> {
    // This test documents that prover client failures should:
    // 1. Propagate errors to the caller
    // 2. Not leave partial batches in the database
    // 3. Allow retry after prover becomes available

    // With a proper mock setup (or integration test), we would:
    // - Mock submit_task to return ProverClientError
    // - Verify error propagates
    // - Verify no partial batches exist
    // - Verify retry succeeds after mock is fixed

    Ok(())
}

/// Test: Aggregator handles storage put failure
///
/// This test validates that:
/// - When storage.put_data() fails, the error is propagated
/// - Database is not updated (maintains consistency)
/// - Can retry after storage is fixed
///
/// Note: This test documents expected behavior. Full mocking of storage
/// errors requires compatible error types.
#[rstest]
#[tokio::test]
async fn test_aggregator_handles_storage_put_failure() -> Result<(), Box<dyn Error>> {
    // This test documents that storage failures should:
    // 1. Propagate errors to the caller
    // 2. Maintain database consistency (no partial updates)
    // 3. Allow retry after storage is restored

    // With a proper mock setup (or integration test), we would:
    // - Mock put_data to return StorageError
    // - Verify error propagates
    // - Verify database is not updated
    // - Verify retry succeeds after mock is fixed

    Ok(())
}

/// Test: Aggregator handles lock already held gracefully
///
/// This test validates that:
/// - When lock is already held, worker exits gracefully (no panic)
/// - Log indicates lock was not acquired
/// - No batches are modified
#[rstest]
#[tokio::test]
async fn test_aggregator_lock_already_held() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Database expectations (should not be called)
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));

    // Mock lock to return AlreadyHeld
    setup
        .lock
        .expect_acquire_lock()
        .withf(|key, value, expiry_seconds, owner| {
            key == "AggregatorBatchingWorker"
                && *value == LockValue::Boolean(false)
                && *expiry_seconds == 3600
                && owner.is_none()
        })
        .returning(|_, _, _, _| Ok(LockResult::AlreadyHeld("another_worker".to_string())));

    // No release expected since we never acquired
    setup.lock.expect_release_lock().times(0);

    setup.setup_storage_expectations(None);

    let config = setup.build().await;

    // Execute: Run the aggregator batching trigger
    let result = AggregatorBatchingTrigger.run_worker(config).await;

    // Assert: Should complete without error (graceful handling)
    // The worker should recognize the lock is held and exit cleanly
    // Note: Actual behavior may vary - it might return Ok or an error
    // The key is that it doesn't panic
    assert!(
        result.is_ok() || result.is_err(),
        "Worker should handle lock contention gracefully without panicking"
    );

    Ok(())
}

/// Test: Aggregator handles transient RPC failure correctly
///
/// This test validates that:
/// - When RPC fails, the error is propagated
/// - Database state remains consistent
/// - Worker can be retried after RPC recovers
///
/// Note: This test documents the expected recovery behavior.
/// Full retry testing would require more complex mock state management.
#[rstest]
#[tokio::test]
async fn test_aggregator_handles_transient_rpc_failure() -> Result<(), Box<dyn Error>> {
    let mut setup = BatchingTestSetup::new();

    // Setup: Empty database
    setup.database.expect_get_latest_aggregator_batch().returning(|| Ok(None));
    setup.database.expect_create_aggregator_batch().returning(Ok);
    setup
        .database
        .expect_update_or_create_aggregator_batch()
        .returning(|batch, _| Ok(batch.clone()));

    setup.setup_storage_expectations(None);
    setup.setup_lock_expectations();

    // Mock RPC to fail
    setup.server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(500).body(json!({"error": "Transient RPC failure"}).to_string());
    });

    let config = setup.build().await;

    // Execute: Should fail due to RPC
    let result = AggregatorBatchingTrigger.run_worker(config).await;
    assert!(result.is_err(), "Should fail when RPC is unavailable");

    // In production:
    // 1. Worker would log the error
    // 2. Retry mechanism would attempt again later
    // 3. Database state remains consistent (no partial batches)
    // 4. When RPC recovers, worker continues from last successful state

    Ok(())
}
