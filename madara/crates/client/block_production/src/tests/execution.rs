use super::*;

// This test makes sure that the preconfirmed tick closes the block
// if the bouncer capacity is reached
#[ignore] // FIXME: this test is complicated by the fact validation / actual execution fee may differ a bit. Ignore for now.
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
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 2 }
    ));
    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

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
async fn test_no_empty_blocks_does_not_close_empty_block(
    #[future]
    #[with(Duration::from_millis(200), false, true)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    let mut block_production_task = devnet_setup.block_prod_task();

    let mut notifications = block_production_task.subscribe_state_notifications();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    assert!(tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await.is_err());
    assert_eq!(devnet_setup.backend.block_view_on_last_confirmed().unwrap().block_number(), 0);
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
    devnet_setup.tx_validator.submit_invoke_transaction(tx.into()).await.unwrap();

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
