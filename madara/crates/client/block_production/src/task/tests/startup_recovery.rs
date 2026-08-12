use super::*;

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
            &setup.contracts.0[1],
            &setup.backend,
            /* nonce */ Felt::ZERO,
            declare_res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        setup.tx_validator.submit_invoke_transaction(deploy_tx.into()).await.unwrap();

        // 3. Invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[2],
            &setup.contracts.0[3],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO,
        )
        .await;

        // 4. Another invoke transaction
        sign_and_add_invoke_tx(
            &setup.contracts.0[4],
            &setup.contracts.0[5],
            &setup.backend,
            &setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 5. Add L1 handler transaction
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

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if original_devnet_setup
                .backend
                .block_view_on_current_preconfirmed()
                .is_some_and(|view| view.num_executed_transactions() == 5)
            {
                break;
            }
            notifications.recv().await.expect("block production notification channel should stay open");
        }
    })
    .await
    .expect("all transaction types should execute before close");

    // Manually close the block
    control.close_block().await.unwrap();
    loop {
        if matches!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 1 }) {
            break;
        }
    }

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

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if restart_devnet_setup
                .backend
                .block_view_on_current_preconfirmed()
                .is_some_and(|view| view.num_executed_transactions() == executed_transactions.len())
            {
                break;
            }
            restart_notifications.recv().await.expect("block production notification channel should stay open");
        }
    })
    .await
    .expect("restart phase should execute the same transaction set");

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

    // Step 5: Now call close_preconfirmed_block_if_exists to re-execute and close the preconfirmed block
    let mut reexec_block_production_task = restart_devnet_setup.block_prod_task();
    reexec_block_production_task.close_preconfirmed_block_if_exists().await.unwrap();

    // Step 6: Verify results match
    assert!(!restart_devnet_setup.backend.has_preconfirmed_block());
    assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(block_number));

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

    // Sort both state diffs to normalize ordering before comparison.
    // TODO(step-5 comparator): move this normalization into a dedicated pure canonicalization
    // function shared by comparator state-diff matching logic.
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

#[rstest::rstest]
#[timeout(Duration::from_secs(100))]
#[tokio::test]
async fn test_startup_recovery_uses_blockifier_output_not_stale_saved_rows(
    #[future]
    #[from(devnet_setup)]
    original_devnet_setup: DevnetSetup,
    #[future]
    #[from(devnet_setup)]
    restart_devnet_setup: DevnetSetup,
) {
    let mut original_devnet_setup = original_devnet_setup.await;
    let mut restart_devnet_setup = restart_devnet_setup.await;

    sign_and_add_invoke_tx(
        &original_devnet_setup.contracts.0[0],
        &original_devnet_setup.contracts.0[1],
        &original_devnet_setup.backend,
        &original_devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut original_block_production_task = original_devnet_setup.block_prod_task();
    let mut original_notifications = original_block_production_task.subscribe_state_notifications();
    let original_control = original_block_production_task.handle();
    let _original_task = AbortOnDrop::spawn(async move {
        original_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    assert_eq!(original_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    original_control.close_block().await.unwrap();
    assert!(matches!(
        original_notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    let block_number = original_devnet_setup.backend.latest_confirmed_block_n().unwrap();
    let expected_block = original_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let expected_txs = expected_block.get_executed_transactions(..).unwrap();
    let mut expected_state_diff = expected_block.get_state_diff().unwrap();

    sign_and_add_invoke_tx(
        &restart_devnet_setup.contracts.0[0],
        &restart_devnet_setup.contracts.0[1],
        &restart_devnet_setup.backend,
        &restart_devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let mut restart_block_production_task = restart_devnet_setup.block_prod_task();
    let mut restart_notifications = restart_block_production_task.subscribe_state_notifications();
    let restart_task = AbortOnDrop::spawn(async move {
        restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
    });

    assert_eq!(restart_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    let preconfirmed_view = restart_devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
    let saved_header = preconfirmed_view.block().header.clone();
    let mut stale_rows: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();
    assert_eq!(stale_rows.len(), 1, "expected one persisted preconfirmed row");

    if let mp_receipt::TransactionReceipt::Invoke(receipt) = &mut stale_rows[0].transaction.receipt {
        receipt.actual_fee.amount = Felt::ZERO;
        receipt.execution_result = mp_receipt::ExecutionResult::Reverted { reason: "stale saved receipt".into() };
    } else {
        panic!("expected invoke receipt in saved row");
    }
    stale_rows[0].state_diff.nonces.insert(Felt::from(0xDEADu64), Felt::from(0xBEEFu64));
    stale_rows[0].state_diff.storage_diffs.insert((Felt::from(0xDEADu64), Felt::from(0x1u64)), Felt::from(0xBADu64));
    let stale_saved_row = stale_rows[0].clone();

    drop(restart_task);
    tokio::time::sleep(Duration::from_millis(200)).await;

    restart_devnet_setup
        .backend
        .write_access()
        .replace_internal_preconfirmed_content_and_persist(block_number, stale_rows)
        .expect("replace persisted rows with stale content");

    let mut recovery_task = restart_devnet_setup.block_prod_task();
    recovery_task.close_preconfirmed_block_if_exists().await.unwrap();

    let recovered_block = restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
    let recovered_block_info = recovered_block.get_block_info().unwrap();
    let recovered_txs = recovered_block.get_executed_transactions(..).unwrap();
    let mut recovered_state_diff = recovered_block.get_state_diff().unwrap();

    assert_eq!(saved_header.block_timestamp, recovered_block_info.header.block_timestamp);
    assert_eq!(saved_header.protocol_version, recovered_block_info.header.protocol_version);
    assert_eq!(saved_header.l1_da_mode, recovered_block_info.header.l1_da_mode);
    assert_eq!(saved_header.gas_prices, recovered_block_info.header.gas_prices);
    assert_eq!(saved_header.sequencer_address, recovered_block_info.header.sequencer_address);
    assert_eq!(saved_header.block_number, recovered_block_info.header.block_number);

    expected_state_diff.sort();
    recovered_state_diff.sort();
    assert_eq!(recovered_txs, expected_txs, "recovered block body must come from Blockifier output");
    assert_eq!(recovered_state_diff, expected_state_diff, "recovered state diff must ignore stale saved rows");
    assert_ne!(
        recovered_txs[0].receipt, stale_saved_row.transaction.receipt,
        "startup recovery must not keep the stale saved receipt"
    );
}
