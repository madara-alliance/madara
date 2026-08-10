use super::*;

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
#[allow(clippy::too_many_arguments)]
async fn test_block_prod_bouncer_cap_reached_closes_block(
    #[future]
    // Use a very very long block time (longer than the test timeout).
    #[with(Duration::from_secs(10000000), true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // The transaction itself is meaningless, it's just to check
    // if the task correctly reads it and process it
    assert!(devnet_setup.mempool.is_empty().await);
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[2],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    assert!(!devnet_setup.mempool.is_empty().await);

    let mut block_production_task = devnet_setup.block_prod_task();
    // The BouncerConfig is set up with amounts (100000) that should limit
    // the block size in a way that the pending tick on this task
    // closes the block
    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    tokio::time::sleep(Duration::from_secs(5)).await;

    tracing::debug!("{:?}", devnet_setup.backend.block_view_on_latest().map(|l| l.get_executed_transactions(..)));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut batch_executed = 0usize;
    let mut closed_blocks = Vec::new();
    while batch_executed < 3 || !closed_blocks.contains(&1) || !closed_blocks.contains(&2) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let notification = tokio::time::timeout(remaining.max(Duration::from_millis(1)), notifications.recv())
            .await
            .expect("expected bouncer-cap notifications before timeout")
            .expect("notification channel should stay open");
        match notification {
            BlockProductionStateNotification::BatchExecuted => batch_executed += 1,
            BlockProductionStateNotification::ClosedBlock { block_n } => closed_blocks.push(block_n),
        }
    }
    assert!(batch_executed >= 3, "expected at least three executed-batch notifications, got {batch_executed}");
    assert!(closed_blocks.contains(&1), "expected block 1 to be closed");
    assert!(closed_blocks.contains(&2), "expected block 2 to be closed");

    let closed_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
    let closed_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
    let preconfirmed_3 = devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
    assert_eq!(preconfirmed_3.block_number(), 3);
    assert_eq!(closed_1.get_executed_transactions(..).unwrap().len(), 1);
    // rolled over to next block.
    assert_eq!(closed_2.get_executed_transactions(..).unwrap().len(), 1);
    // rolled over to next block.
    // last block should not be closed though.
    assert_eq!(preconfirmed_3.get_executed_transactions(..).len(), 1);
    assert!(devnet_setup.mempool.is_empty().await);
}

// This test makes sure that the block time tick correctly
// adds the transaction to the preconfirmed block, closes it
// and creates a new empty preconfirmed block
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
#[allow(clippy::too_many_arguments)]
async fn test_block_prod_on_block_time_tick_closes_block(
    #[future]
    #[with(Duration::from_secs(2), true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // The block should be closed after 3s.
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    let view = devnet_setup.backend.block_view_on_last_confirmed().unwrap();

    assert_eq!(view.block_number(), 1);
    assert_eq!(view.get_executed_transactions(..).unwrap(), []);
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_l1_handler_tx(
    #[future]
    #[with(Duration::from_secs(3000000000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;
    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Declare the contract class.
    let res = sign_and_add_declare_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        /* nonce */ Felt::ZERO,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(
        devnet_setup
            .backend
            .block_view_on_current_preconfirmed()
            .unwrap()
            .get_executed_transaction(0)
            .unwrap()
            .receipt
            .execution_result(),
        ExecutionResult::Succeeded
    );
    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    // Deploy contract through UDC.

    let (contract_address, tx) = make_udc_call(
        &devnet_setup.contracts.0[0],
        &devnet_setup.backend,
        /* nonce */ Felt::ONE,
        res.class_hash,
        /* calldata (pubkey) */ &[Felt::TWO],
    );
    devnet_setup.tx_validator.submit_invoke_transaction(tx).await.unwrap();

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    assert_eq!(
        devnet_setup
            .backend
            .block_view_on_current_preconfirmed()
            .unwrap()
            .get_executed_transaction(0)
            .unwrap()
            .receipt
            .execution_result(),
        ExecutionResult::Succeeded
    );

    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 2 }
    ));

    // Mock the l1 message, block prod should pick it up.

    devnet_setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            version: Felt::ZERO,
            nonce: 55, // core contract nonce
            contract_address,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: vec![/* from_address */ Felt::THREE, /* arg1 */ Felt::ONE, /* arg2 */ Felt::TWO].into(),
        },
        /* paid_fee_on_l1 */ 128328,
    ));

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    let receipt =
        devnet_setup.backend.block_view_on_current_preconfirmed().unwrap().get_executed_transaction(0).unwrap().receipt;
    assert_eq!(receipt.execution_result(), ExecutionResult::Succeeded);
    tracing::info!("Events = {:?}", receipt.events());
    assert_eq!(receipt.events().len(), 1);

    assert_eq!(
        receipt.events()[0],
        Event {
            from_address: contract_address,
            keys: vec![get_selector_from_name("CalledFromL1").unwrap()],
            data: vec![/* from_address */ Felt::THREE, /* arg1 */ Felt::ONE, /* arg2 */ Felt::TWO]
        }
    );
}

/// Verifies that re-execution uses the saved `no_charge_fee` value.
///
/// # Flow
/// 1. **Initial**: `no_charge_fee = true`. Exec tx, stop before closing. Saved: `true`.
/// 2. **Restart**: `no_charge_fee = false`.
/// 3. **Re-execution**: Uses saved `true` value. Receipts match.
/// 4. **Post**: Config updates to `false` for next block.
#[rstest::rstest]
#[timeout(Duration::from_secs(100))]
#[tokio::test]
async fn test_reexecution_uses_saved_no_charge_fee_value(
    #[future]
    #[from(devnet_setup)]
    original_devnet_setup: DevnetSetup,
) {
    let original_devnet_setup = original_devnet_setup.await;

    // Phase 1: Initial execution with no_charge_fee = true
    let initial_no_charge_fee = true;
    assert!(original_devnet_setup.mempool.is_empty().await);

    // Create a transaction validator that matches our no_charge_fee setting.
    // This ensures transactions are validated with charge_fee = !no_charge_fee.
    // Without this, transactions would be validated with charge_fee = true (default),
    // causing a mismatch between validation and execution.
    let tx_validator_with_no_fee = Arc::new(TransactionValidator::new(
        Arc::clone(&original_devnet_setup.mempool) as _,
        Arc::clone(&original_devnet_setup.backend),
        TransactionValidatorConfig { disable_validation: false, disable_fee: initial_no_charge_fee },
    ));

    sign_and_add_invoke_tx(
        &original_devnet_setup.contracts.0[0],
        &original_devnet_setup.contracts.0[1],
        &original_devnet_setup.backend,
        &tx_validator_with_no_fee,
        Felt::ZERO,
    )
    .await;

    assert!(!original_devnet_setup.mempool.is_empty().await);

    // Start block production task with no_charge_fee = true.
    // This will execute the transaction and add it to the pre-confirmed block.
    let mut block_production_task = BlockProductionTask::new(
        original_devnet_setup.backend.clone(),
        original_devnet_setup.mempool.clone(),
        original_devnet_setup.metrics.clone(),
        Arc::new(original_devnet_setup.l1_client.clone()),
        false, // mempool_paused
        initial_no_charge_fee,
    );

    let mut notifications = block_production_task.subscribe_state_notifications();
    let restart_task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Wait for transaction to be executed and added to pre-confirmed block
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Verify pre-confirmed block exists with our transaction
    assert!(original_devnet_setup.backend.has_preconfirmed_block());
    let preconfirmed_view = original_devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

    // Stop the task before it closes the block.
    // This simulates a node crash/restart scenario where a pre-confirmed block exists.
    drop(restart_task);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Phase 2: Restart with different no_charge_fee value
    // This simulates a configuration change between shutdown and restart.
    let restart_no_charge_fee = false;
    let restart_block_production_task = BlockProductionTask::new(
        original_devnet_setup.backend.clone(), // Same backend = same database
        original_devnet_setup.mempool.clone(),
        original_devnet_setup.metrics.clone(),
        Arc::new(original_devnet_setup.l1_client.clone()),
        false,                 // mempool_paused
        restart_no_charge_fee, // Current config: no_charge_fee = false
    );

    // Start the block production task.
    // This will call setup_initial_state() which calls close_preconfirmed_block_if_exists().
    // During re-execution, it will use saved_no_charge_fee = true (from saved config),
    // NOT restart_no_charge_fee = false (from current config).
    let _restart_task = AbortOnDrop::spawn(async move {
        restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    // Give time for setup_initial_state to complete and close the pre-confirmed block
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Phase 3: Verify block was closed successfully
    assert!(!original_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(original_devnet_setup.backend.latest_confirmed_block_n(), Some(1));

    // Phase 4: Verify config was updated with CURRENT value after re-execution
    // After re-execution completes, the config is updated to the current value.
    // This ensures that the next block will use the current configuration.
    let updated_config = original_devnet_setup
        .backend
        .get_runtime_exec_config()
        .expect("Should be able to read runtime exec config")
        .expect("Runtime exec config should exist after closing");

    assert_eq!(
        updated_config.no_charge_fee, restart_no_charge_fee,
        "Config should be updated with current value after re-execution completes"
    );
}

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

/// C-006D / C-010B: Integration test proving that a comparator pipeline error during block
/// close does NOT silently canonicalize from speculative EB output. The block production
/// task must fail, and the block must remain preconfirmed for crash recovery.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_comparator_pipeline_error_forces_failsafe_fallback(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::StartupExecutionMode;

    let devnet_setup = devnet_setup.await;

    // Submit a transaction so the block has content when close_block runs.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    assert!(!devnet_setup.mempool.is_empty().await);

    // Build task in Mixed startup mode with comparator error failpoint active.
    let mut block_production_task = BlockProductionTask::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        devnet_setup.metrics.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        false, // mempool_paused
        false, // no_charge_fee
    )
    .with_startup_execution_mode(StartupExecutionMode::Mixed)
    .with_test_force_comparator_error();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();

    // C-010B: Run the task and capture the result (do NOT unwrap — we expect failure).
    let task = tokio::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await });

    // Wait for the batch to be executed.
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Close the block — this triggers the comparator in Mixed mode, which will hit
    // the force_comparator_error failpoint. C-010B: the task must fail, not silently
    // close the block from speculative EB output.
    control.close_block().await.unwrap();

    // The task must fail with the canonical output unavailable error.
    let task_result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("task should complete, not hang")
        .expect("task join should not panic");
    assert!(task_result.is_err(), "block production must fail on comparator pipeline error");
    let err_msg = format!("{:#}", task_result.unwrap_err());
    assert!(
        err_msg.contains("Canonical output unavailable"),
        "error must mention canonical output unavailable, got: {err_msg}"
    );

    // C-010B: The block must remain preconfirmed (NOT closed from speculative EB).
    assert!(devnet_setup.backend.has_preconfirmed_block(), "block must remain preconfirmed for crash recovery");
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn manual_disable_mid_block_keeps_current_block_mixed_for_canonicalization(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};

    let devnet_setup = devnet_setup.await;

    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut block_production_task = BlockProductionTask::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        devnet_setup.metrics.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        false,
        false,
    )
    .with_startup_execution_mode(StartupExecutionMode::Mixed)
    .with_test_force_comparator_error();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let task = tokio::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await });

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(
        control.executionbox_status().await.unwrap().mode,
        ExecutionMode::Mixed,
        "block should start in Mixed mode"
    );

    control.executionbox_disable().await.unwrap();
    assert_eq!(
        control.executionbox_status().await.unwrap().mode,
        ExecutionMode::BlockifierOnly,
        "manual disable should change desired mode for future blocks"
    );

    control.close_block().await.unwrap();

    let task_result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("task should complete, not hang")
        .expect("task join should not panic");
    assert!(task_result.is_err(), "mixed block should still hit the comparator failpoint after manual disable");
    let err_msg = format!("{:#}", task_result.unwrap_err());
    assert!(
        err_msg.contains("Canonical output unavailable"),
        "error must mention canonical output unavailable, got: {err_msg}"
    );
    assert!(
        devnet_setup.backend.has_preconfirmed_block(),
        "failed mixed canonicalization must leave the block preconfirmed"
    );
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn manual_enable_mid_block_only_applies_to_next_block(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::manager::EnableOutcome;
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};

    let devnet_setup = devnet_setup.await;

    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut block_production_task = BlockProductionTask::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        devnet_setup.metrics.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        false,
        false,
    )
    .with_startup_execution_mode(StartupExecutionMode::BlockifierOnly)
    .with_test_force_comparator_error();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let task = tokio::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await });

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(
        control.executionbox_status().await.unwrap().mode,
        ExecutionMode::BlockifierOnly,
        "startup mode should keep the first block BlockifierOnly"
    );

    let enable_result = control.executionbox_enable().await.unwrap().expect("manual enable should succeed");
    assert_eq!(enable_result, EnableOutcome::EnabledNow);
    assert_eq!(
        control.executionbox_status().await.unwrap().mode,
        ExecutionMode::Mixed,
        "manual enable should set the desired mode for future blocks"
    );

    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ONE,
    )
    .await;

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert_eq!(
        control.executionbox_status().await.unwrap().mode,
        ExecutionMode::Mixed,
        "manual enable should take effect on the next block"
    );

    control.close_block().await.unwrap();

    let task_result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("task should complete, not hang")
        .expect("task join should not panic");
    assert!(task_result.is_err(), "the second block should hit the comparator failpoint once Mixed is active");
    let err_msg = format!("{:#}", task_result.unwrap_err());
    assert!(
        err_msg.contains("Canonical output unavailable"),
        "error must mention canonical output unavailable, got: {err_msg}"
    );
    assert!(
        devnet_setup.backend.has_preconfirmed_block(),
        "the failed mixed canonicalization on the second block must leave the block preconfirmed"
    );
}
