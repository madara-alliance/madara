use super::*;

// This test verifies that graceful shutdown properly closes any open preconfirmed block
// without requiring re-execution. When shutdown is triggered, the block production service
// should close the preconfirmed block using the executor's existing state.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_graceful_shutdown_closes_preconfirmed_block(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Step 1: Set up block production with transactions
    assert!(devnet_setup.mempool.is_empty().await);

    // Add a transaction to the mempool
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert!(!devnet_setup.mempool.is_empty().await);

    // Step 2: Start block production and execute a batch to create a preconfirmed block
    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();

    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

    // Wait for batch to be executed (transactions added to preconfirmed block)
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Verify preconfirmed block exists with transactions
    assert!(devnet_setup.backend.has_preconfirmed_block());
    let preconfirmed_view = devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

    // Step 3: Trigger graceful shutdown by cancelling ServiceContext
    ctx_clone.cancel_global();

    // Step 4: Wait for EndFinalBlock to be processed (indicated by ClosedBlock notification)
    // During graceful shutdown:
    // - Batcher detects cancellation and exits, closing the send_batch channel
    // - Executor detects channel closure and sends EndFinalBlock message
    // - Main loop processes EndFinalBlock and closes the block (sends ClosedBlock notification)
    assert!(
        matches!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 1 }),
        "Expected ClosedBlock notification after EndFinalBlock was processed during graceful shutdown"
    );

    // Step 5: Wait for shutdown to complete
    // All database writes and head projection updates complete synchronously within the awaited rayon task,
    // so by the time task.await completes, the state is already updated. No delay needed.
    task.await.unwrap();

    // Step 6: Verify the preconfirmed block is closed and saved to database
    assert!(!devnet_setup.backend.has_preconfirmed_block(), "Preconfirmed block should be closed");

    // Verify block was properly closed (check latest confirmed block number)
    let latest_block_n = devnet_setup.backend.latest_confirmed_block_n();
    assert!(latest_block_n.is_some(), "Block should be closed and saved");
    let block_number = latest_block_n.unwrap();

    // Verify transactions are preserved correctly
    let closed_block = devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let executed_transactions = closed_block.get_executed_transactions(..).unwrap();
    assert_eq!(executed_transactions.len(), 1, "Transaction should be preserved in closed block");

    // Verify mempool is empty (transaction was consumed)
    assert!(devnet_setup.mempool.is_empty().await);
}

// This test verifies that graceful shutdown completes successfully when there is no
// preconfirmed block to close. The shutdown should complete without errors.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_graceful_shutdown_with_no_preconfirmed_block(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Step 1: Start block production without adding any transactions
    // This ensures no preconfirmed block is created
    assert!(devnet_setup.mempool.is_empty().await);
    assert!(!devnet_setup.backend.has_preconfirmed_block());

    let block_production_task = devnet_setup.block_prod_task();
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();

    let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

    // Step 2: Give a small delay to ensure block production task is running
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Step 3: Verify no preconfirmed block exists
    assert!(!devnet_setup.backend.has_preconfirmed_block());

    // Step 4: Trigger graceful shutdown immediately
    ctx_clone.cancel_global();

    // Step 5: Wait for shutdown to complete - should complete without errors
    // Since there's no preconfirmed block, shutdown should complete immediately
    // without waiting for EndBlock
    task.await.unwrap();

    // Step 6: Verify shutdown completed successfully
    // No preconfirmed block should exist (still)
    assert!(!devnet_setup.backend.has_preconfirmed_block());
}

// Regression test: when a non-empty block is followed by an empty block,
// the timestamp delta should be ~block_time, not ~2*block_time.
//
// Before the fix, create_execution_context() called SystemTime::now() lazily
// — only after wait_take_tx_batch() returned. For a non-empty block, txs
// arrive quickly so the timestamp is set near block-open time. For an empty
// block, the full block_time elapses before the timestamp is set.
// This made the delta between a non-empty and subsequent empty block ≈ 2*block_time.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_empty_block_timestamp_not_drifted(
    #[future]
    #[with(Duration::from_secs(3))]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Submit a transaction so block 1 is non-empty.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Block 1: non-empty (has our tx), closes after block_time.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 1 });

    // Block 2: empty, closes after another block_time.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 2 });

    let block_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let block_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();

    let ts_1 = block_1.get_block_info().unwrap().header.block_timestamp.0;
    let ts_2 = block_2.get_block_info().unwrap().header.block_timestamp.0;

    let delta = ts_2.saturating_sub(ts_1);

    // With block_time=3s, the delta should be ~3s.
    // Before the fix it would be ~6s (2 * block_time) because:
    //   - block 1 timestamp set at open (near T0)
    //   - block 2 timestamp set after 3s wait (near T0 + 3s + 3s)
    assert!(
        delta >= 2,
        "Timestamp delta between non-empty and subsequent empty block should be ~3s (block_time), \
             but got {delta}s. Timestamps may have stalled or gone backward."
    );
    assert!(
        delta <= 4,
        "Timestamp delta between non-empty and subsequent empty block should be ~3s (block_time), \
             but got {delta}s. This likely means the timestamp is still being set after the block_time wait."
    );
}

// When no_empty_blocks=true, blocks are produced on-demand. The timestamp
// should reflect wall-clock time when the first tx arrives, not the time
// the previous block closed (which could be arbitrarily long ago).
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_no_empty_blocks_timestamp_uses_wall_clock(
    #[future]
    #[with(Duration::from_secs(30), false, true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Submit a tx to trigger block 1.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 1 });

    // Wait 3 seconds before submitting the next tx. With no_empty_blocks=true,
    // the executor waits indefinitely. The block timestamp should reflect
    // when the tx arrives (~3s from now), not when block 1 closed (~3s ago).
    tokio::time::sleep(Duration::from_secs(3)).await;

    let wall_clock_before_tx = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 2 });

    let block_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
    let ts_2 = block_2.get_block_info().unwrap().header.block_timestamp.0;

    // The timestamp should be within 1s of wall clock when the tx was submitted,
    // not ~3s behind (which would indicate the stale captured time was used).
    let drift = wall_clock_before_tx.saturating_sub(ts_2);
    assert!(
        drift <= 1,
        "With no_empty_blocks=true, block timestamp should reflect wall-clock time \
             when the first tx arrived, but it was {drift}s behind. \
             This likely means a stale captured block_start_time was used."
    );
}
