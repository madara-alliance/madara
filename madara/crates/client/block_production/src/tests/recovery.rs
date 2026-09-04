use super::*;

//
// This test verifies that when Madara restarts with a preconfirmed block, `close_preconfirmed_block_if_exists`
// correctly re-executes transactions and produces the same global state root, state diff, and receipts as the original
// execution. This ensures correctness of the restart recovery mechanism.
//
// # Test Process
//
// **Phase 1: Normal Block Production**
// 1. Creates a block with various transaction types (invoke, declare, deploy, L1 handler)
// 2. Closes the block normally and captures:
//    - `global_state_root`
//    - `state_diff`
//    - `header` information
//    - Executed transactions
//
// # Transaction Types Tested
// - **Invoke transactions**: Standard contract calls
// - **Declare transactions**: Class declarations
// - **Deploy transactions**: Contract deployments via UDC
// - **L1 handler transactions**: L1 to L2 messages with `paid_fee_on_l1`
//
// # Key Assertions
//
// - Global state root must match exactly (ensures state consistency)
// - State diff must match (values are the same, order may differ)
// - Header fields must match the preconfirmed block (timestamp, gas_prices, etc.)
// - All transactions must match
// - All receipts must match exactly (ensures execution results are identical)
//
// # Important Notes
//
// - Uses two separate `DevnetSetup` fixtures to ensure clean state isolation
// - State diffs are sorted before comparison to handle ordering differences
// - The test verifies that `paid_fee_on_l1` is preserved for L1 handler transactions
// - The test ensures that re-execution produces deterministic results
#[rstest::rstest]
#[timeout(Duration::from_secs(100))]
#[tokio::test]
async fn test_close_preconfirmed_block_reexecution_matches_normal_closing(
    #[future]
    #[from(devnet_setup)]
    original_devnet_setup: DevnetSetup,
    #[future]
    #[from(devnet_setup)]
    restart_devnet_setup: DevnetSetup,
) {
    // used for phase 1, where we close the block and note down its
    // global_state_root, state_diff, and header info
    let mut original_devnet_setup = original_devnet_setup.await;

    // use for phase 2, where we compare the state of the block after re-execution with the state of the block before re-execution
    let mut restart_devnet_setup = restart_devnet_setup.await;

    // --------------------------------------------------------------
    // | PHASE 1: Close the block and note down its state.          |
    // --------------------------------------------------------------

    // Step 1: Create a block normally with transactions in the original backend
    assert!(original_devnet_setup.mempool.is_empty().await);

    // Helper function to create and execute transactions for testing
    async fn create_and_execute_transactions(setup: &DevnetSetup) -> Felt {
        // 1. Declare a contract
        let declare_res =
            sign_and_add_declare_tx(&setup.contracts.0[0], &setup.backend, &setup.tx_validator, Felt::ZERO).await;

        // 2. Deploy contract through UDC
        let (contract_address, deploy_tx) = make_udc_call(
            &setup.contracts.0[0],
            &setup.backend,
            /* nonce */ Felt::ONE,
            declare_res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        setup.tx_validator.submit_invoke_transaction(deploy_tx.into()).await.unwrap();

        // 3. Invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[0],
            &setup.contracts.0[1],
            &setup.backend,
            &setup.tx_validator,
            Felt::TWO, // nonce after declare (ZERO) and deploy (ONE)
        )
        .await;

        // 4. Declare transaction (for a different contract)
        sign_and_add_declare_tx(
            &setup.contracts.0[2],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 5. Another invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[1],
            &setup.contracts.0[3],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 6. Add L1 handler transaction
        let paid_fee_on_l1 = 128328u128;
        setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 55, // core contract nonce
                contract_address,
                entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
                calldata: vec![
                    /* from_address */ Felt::THREE,
                    /* arg1 */ Felt::ONE,
                    /* arg2 */ Felt::TWO,
                ]
                .into(),
            },
            paid_fee_on_l1,
        ));

        contract_address
    }

    // Add various transaction types to mempool to test re-execution handles all types correctly
    // All transactions will be in a single block
    let _contract_address = create_and_execute_transactions(&original_devnet_setup).await;

    assert!(!original_devnet_setup.mempool.is_empty().await);

    // Run block production to create and close a block with all transactions
    let mut block_production_task = original_devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    // Wait for batch to be executed
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Manually close the block
    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    // Step 2: Capture global_state_root, state_diff, and header info from closed block
    let block_number = original_devnet_setup.backend.latest_confirmed_block_n().unwrap();
    let original_block = original_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let original_block_info = original_block.get_block_info().unwrap();
    let expected_global_state_root = original_block_info.header.global_state_root;
    let expected_state_diff = original_block.get_state_diff().unwrap();
    let executed_transactions = original_block.get_executed_transactions(..).unwrap();

    // --------------------------------------------------------------
    // | PHASE 2: Re-execute the block and note down its state.    |
    // --------------------------------------------------------------
    //
    // We'll add them in the same order using the same helper functions
    // All transactions will be in a single block
    // This ensures they're executed in the same context (clean genesis state)
    assert!(restart_devnet_setup.mempool.is_empty().await);

    // Create the same transactions using the helper function
    let _restart_contract_address = create_and_execute_transactions(&restart_devnet_setup).await;

    assert!(!restart_devnet_setup.mempool.is_empty().await);

    // Step 4: Run block production to execute transactions and add them to preconfirmed block
    // Use a very long block_time to prevent auto-closing, then stop manually after batch execution
    let mut restart_block_production_task = restart_devnet_setup.block_prod_task();
    let mut restart_notifications = restart_block_production_task.subscribe_state_notifications();
    let restart_task = AbortOnDrop::spawn(async move {
        restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    // Wait for batch to be executed (transactions added to preconfirmed block)
    assert_eq!(restart_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    // Fetch preconfirmed block view BEFORE dropping the task to avoid race conditions
    let preconfirmed_view = restart_devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
    assert_eq!(preconfirmed_view.num_executed_transactions(), executed_transactions.len());
    let restart_preconfirmed_block = preconfirmed_view.block();

    // Stop the task before it closes the block (drop the AbortOnDrop which will abort the task)
    drop(restart_task);

    // Give it a moment to finish current operations
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify preconfirmed block still exists with transactions and no confirmed blocks yet
    assert!(restart_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(0));

    // adding some delay to see if block_timestamp would differ in the reexecution or not
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Step 5: Run parallel-Merkle startup recovery. Besides re-executing and closing the
    // preconfirmed block, setup must publish the recovered head as the next durable root base.
    let mut reexec_block_production_task = restart_devnet_setup.block_prod_task().with_parallel_merkle_enabled(true);
    reexec_block_production_task.setup_initial_state().await.unwrap();

    // Step 6: Verify results match
    assert!(!restart_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(block_number));
    assert_eq!(
        restart_devnet_setup.backend.db.get_parallel_merkle_latest_checkpoint().unwrap(),
        Some(block_number),
        "recovered head must be the durable base for the first new parallel root"
    );
    assert_eq!(
        restart_devnet_setup
            .backend
            .db
            .get_latest_durable_snapshot_floor(Some(block_number))
            .map(|(snapshot_block, _)| snapshot_block),
        Some(Some(block_number)),
        "recovered head snapshot must be available before normal production starts"
    );

    let reexecuted_block_info =
        restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap().get_block_info().unwrap();

    // Verify the header fields match the pre-execution pre-confirmed block's header
    assert_eq!(restart_preconfirmed_block.header.block_timestamp, reexecuted_block_info.header.block_timestamp);
    assert_eq!(restart_preconfirmed_block.header.protocol_version, reexecuted_block_info.header.protocol_version);
    assert_eq!(restart_preconfirmed_block.header.l1_da_mode, reexecuted_block_info.header.l1_da_mode);
    assert_eq!(restart_preconfirmed_block.header.gas_prices, reexecuted_block_info.header.gas_prices);
    assert_eq!(restart_preconfirmed_block.header.sequencer_address, reexecuted_block_info.header.sequencer_address);
    assert_eq!(restart_preconfirmed_block.header.block_number, reexecuted_block_info.header.block_number);

    let reexecuted_block = restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let reexecuted_block_info = reexecuted_block.get_block_info().unwrap();
    let actual_global_state_root = reexecuted_block_info.header.global_state_root;
    let mut actual_state_diff = reexecuted_block.get_state_diff().unwrap();
    let mut expected_state_diff_sorted = expected_state_diff.clone();

    // Sort both state diffs to normalize ordering before comparison
    actual_state_diff.sort();
    expected_state_diff_sorted.sort();

    // Verify global state root matches
    assert_eq!(
        actual_global_state_root, expected_global_state_root,
        "Global state root should match between normal execution and re-execution"
    );

    // Verify state diff matches (after sorting to ignore ordering differences)
    assert_eq!(
            actual_state_diff, expected_state_diff_sorted,
            "State diff should match between normal execution and re-execution (values are the same, only order may differ)"
        );

    // Verify transactions match
    let reexecuted_transactions = reexecuted_block.get_executed_transactions(..).unwrap();
    assert_eq!(reexecuted_transactions, executed_transactions, "Transactions should match");

    // Verify receipts match - re-execution should produce identical receipts
    assert_eq!(executed_transactions.len(), reexecuted_transactions.len(), "Number of transactions should match");
    for (i, (original_tx, reexecuted_tx)) in
        executed_transactions.iter().zip(reexecuted_transactions.iter()).enumerate()
    {
        assert_eq!(
            original_tx.receipt.transaction_hash(),
            reexecuted_tx.receipt.transaction_hash(),
            "Receipt transaction hash should match for transaction {}",
            i
        );
        assert_eq!(
            original_tx.receipt,
            reexecuted_tx.receipt,
            "Receipt should match exactly for transaction {} (hash: {:#x})",
            i,
            original_tx.receipt.transaction_hash()
        );
    }
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
        false,
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
        false,
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

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_discard_preconfirmed_on_startup_replaces_runtime_exec_config(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    let devnet_setup = devnet_setup.await;

    let initial_no_charge_fee = true;
    let chain_config = devnet_setup.backend.chain_config();
    let exec_constants = chain_config.exec_constants_by_protocol_version(chain_config.latest_protocol_version).unwrap();
    let saved_runtime_config =
        RuntimeExecutionConfig::from_current_config(chain_config, exec_constants, initial_no_charge_fee).unwrap();

    devnet_setup.backend.write_access().write_runtime_exec_config(&saved_runtime_config).unwrap();
    devnet_setup
        .backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new_with_content(
            PreconfirmedHeader { block_number: 1, ..Default::default() },
            vec![PreconfirmedExecutedTransaction {
                transaction: l1_handler_tx_with_receipt(55, Felt::from(0x1234_u64)),
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: Some(0),
            }],
            [],
        ))
        .unwrap();

    assert!(devnet_setup.backend.has_preconfirmed_block());

    let current_no_charge_fee = false;
    let mut restart_block_production_task = BlockProductionTask::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        devnet_setup.metrics.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        false,
        current_no_charge_fee,
        true,
    );

    restart_block_production_task.setup_initial_state().await.unwrap();

    assert!(!devnet_setup.backend.has_preconfirmed_block(), "Preconfirmed block should be discarded on startup");
    assert_eq!(
        devnet_setup.backend.latest_confirmed_block_n(),
        Some(0),
        "Discarding startup recovery should keep the latest confirmed block unchanged"
    );

    let updated_config = devnet_setup
        .backend
        .get_runtime_exec_config()
        .expect("Should be able to read runtime exec config")
        .expect("Runtime exec config should exist after discarding");

    assert_eq!(
        updated_config.no_charge_fee, current_no_charge_fee,
        "Discarding startup recovery should replace the saved runtime config with the current one"
    );
}
