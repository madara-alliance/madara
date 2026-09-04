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
struct RecoveryReference {
    block_number: u64,
    state_root: Felt,
    state_diff: StateDiff,
    transactions: Vec<mp_block::TransactionWithReceipt>,
}

/// Submits the representative declare, deploy, invoke, and L1-handler workload.
/// Reusing this sequence ensures normal closing and restart recovery receive identical inputs.
async fn create_recovery_transactions(setup: &DevnetSetup) {
    let declare_res =
        sign_and_add_declare_tx(&setup.contracts.0[0], &setup.backend, &setup.tx_validator, Felt::ZERO).await;
    let (contract_address, deploy_tx) =
        make_udc_call(&setup.contracts.0[0], &setup.backend, Felt::ONE, declare_res.class_hash, &[Felt::TWO]);
    setup.tx_validator.submit_invoke_transaction(deploy_tx.into()).await.unwrap();
    sign_and_add_invoke_tx(
        &setup.contracts.0[0],
        &setup.contracts.0[1],
        &setup.backend,
        &setup.tx_validator,
        Felt::TWO,
    )
    .await;
    sign_and_add_declare_tx(&setup.contracts.0[2], &setup.backend, &setup.tx_validator, Felt::ZERO).await;
    sign_and_add_invoke_tx(
        &setup.contracts.0[1],
        &setup.contracts.0[3],
        &setup.backend,
        &setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            version: Felt::ZERO,
            nonce: 55,
            contract_address,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: vec![Felt::THREE, Felt::ONE, Felt::TWO].into(),
        },
        128328,
    ));
}

/// Produces and closes the reference block through the normal block-production path.
/// The captured root, diff, and receipts form the oracle for restart recovery.
async fn close_reference_block(setup: &mut DevnetSetup) -> RecoveryReference {
    assert!(setup.mempool.is_empty().await);
    create_recovery_transactions(setup).await;
    assert!(!setup.mempool.is_empty().await);

    let mut task = setup.block_prod_task();
    let mut notifications = task.subscribe_state_notifications();
    let control = task.handle();
    let _task = AbortOnDrop::spawn(async move { task.run(ServiceContext::new_for_testing()).await.unwrap() });
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    let block_number = setup.backend.latest_confirmed_block_n().unwrap();
    let block = setup.backend.block_view_on_confirmed(block_number).unwrap();
    RecoveryReference {
        block_number,
        state_root: block.get_block_info().unwrap().header.global_state_root,
        state_diff: block.get_state_diff().unwrap(),
        transactions: block.get_executed_transactions(..).unwrap(),
    }
}

/// Executes the same workload into a durable preconfirmed block and then aborts production.
/// The returned block captures the header that startup recovery must preserve.
async fn leave_preconfirmed_block(setup: &mut DevnetSetup, expected_transactions: usize) -> Arc<PreconfirmedBlock> {
    assert!(setup.mempool.is_empty().await);
    create_recovery_transactions(setup).await;
    assert!(!setup.mempool.is_empty().await);

    let mut task = setup.block_prod_task();
    let mut notifications = task.subscribe_state_notifications();
    let running = AbortOnDrop::spawn(async move { task.run(ServiceContext::new_for_testing()).await.unwrap() });
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    let view = setup.backend.block_view_on_current_preconfirmed().unwrap();
    assert_eq!(view.num_executed_transactions(), expected_transactions);
    let block = Arc::clone(view.block());
    drop(running);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(setup.backend.has_preconfirmed_block());
    assert_eq!(setup.backend.latest_confirmed_block_n(), Some(0));
    block
}

/// Compares recovered durable state and receipts with the normally closed reference block.
/// Header fields are compared against the exact preconfirmed header observed before restart.
fn assert_recovered_block(setup: &DevnetSetup, reference: RecoveryReference, preconfirmed: &PreconfirmedBlock) {
    assert!(!setup.backend.has_preconfirmed_block());
    assert_eq!(setup.backend.latest_confirmed_block_n(), Some(reference.block_number));
    assert_eq!(
        setup.backend.db.get_parallel_merkle_latest_checkpoint().unwrap(),
        Some(reference.block_number),
        "recovered head must be the durable base for the first new parallel root"
    );
    assert_eq!(
        setup
            .backend
            .db
            .get_latest_durable_snapshot_floor(Some(reference.block_number))
            .map(|(snapshot_block, _)| snapshot_block),
        Some(Some(reference.block_number)),
        "recovered head snapshot must be available before normal production starts"
    );

    let block = setup.backend.block_view_on_confirmed(reference.block_number).unwrap();
    let info = block.get_block_info().unwrap();
    assert_eq!(preconfirmed.header.block_timestamp, info.header.block_timestamp);
    assert_eq!(preconfirmed.header.protocol_version, info.header.protocol_version);
    assert_eq!(preconfirmed.header.l1_da_mode, info.header.l1_da_mode);
    assert_eq!(preconfirmed.header.gas_prices, info.header.gas_prices);
    assert_eq!(preconfirmed.header.sequencer_address, info.header.sequencer_address);
    assert_eq!(preconfirmed.header.block_number, info.header.block_number);
    assert_eq!(info.header.global_state_root, reference.state_root);

    let mut actual_state_diff = block.get_state_diff().unwrap();
    let mut expected_state_diff = reference.state_diff;
    actual_state_diff.sort();
    expected_state_diff.sort();
    assert_eq!(actual_state_diff, expected_state_diff);

    let recovered_transactions = block.get_executed_transactions(..).unwrap();
    assert_eq!(recovered_transactions, reference.transactions);
    for (index, (original, recovered)) in reference.transactions.iter().zip(recovered_transactions.iter()).enumerate() {
        assert_eq!(original.receipt.transaction_hash(), recovered.receipt.transaction_hash());
        assert_eq!(
            original.receipt,
            recovered.receipt,
            "receipt {index} should match for transaction {:#x}",
            original.receipt.transaction_hash()
        );
    }
}

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
    let mut original_setup = original_devnet_setup.await;
    let mut restart_setup = restart_devnet_setup.await;
    let reference = close_reference_block(&mut original_setup).await;
    let preconfirmed = leave_preconfirmed_block(&mut restart_setup, reference.transactions.len()).await;

    let mut recovery_task = restart_setup.block_prod_task().with_parallel_merkle_enabled(true);
    recovery_task.setup_initial_state().await.unwrap();
    assert_recovered_block(&restart_setup, reference, &preconfirmed);
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
