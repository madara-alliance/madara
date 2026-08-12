use super::*;
use mc_devnet::RUST_EXEC_TRANSFER_CONTRACT_ADDRESS;
use starknet_api::abi::abi_utils::get_storage_var_address;

async fn submit_rust_exec_transfer(setup: &DevnetSetup, nonce: Felt, entrypoint: &str, amount: Felt) -> Felt {
    let tx = make_invoke_tx(
        &setup.contracts.0[0],
        Multicall::default().with(Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: Selector::from(entrypoint),
            calldata: vec![setup.contracts.0[1].address, amount],
        }),
        &setup.backend,
        nonce,
    );
    setup
        .tx_validator
        .submit_invoke_transaction(tx.into())
        .await
        .expect("fixture transaction should be accepted")
        .transaction_hash
}

async fn wait_for_confirmed_block(backend: &Arc<MadaraBackend>, block_n: u64) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while backend.latest_confirmed_block_n().is_none_or(|tip| tip < block_n) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("block #{block_n} was not confirmed"));
}

fn block_storage_value(backend: &Arc<MadaraBackend>, block_n: u64, key: Felt) -> Felt {
    backend
        .block_view_on_confirmed(block_n)
        .unwrap_or_else(|| panic!("confirmed block #{block_n} should exist"))
        .get_state_diff()
        .expect("confirmed state diff should be readable")
        .storage_diffs
        .into_iter()
        .find(|item| item.address == RUST_EXEC_TRANSFER_CONTRACT_ADDRESS)
        .and_then(|item| item.storage_entries.into_iter().find(|entry| entry.key == key))
        .unwrap_or_else(|| panic!("fixture storage key {key:#x} missing from block #{block_n}"))
        .value
}

fn block_contains_tx(backend: &Arc<MadaraBackend>, block_n: u64, tx_hash: Felt) -> bool {
    let Some(view) = backend.block_view_on_confirmed(block_n) else { return false };
    view.get_executed_transactions(..).is_ok_and(|txs| txs.iter().any(|tx| *tx.receipt.transaction_hash() == tx_hash))
}

async fn recv_pipeline_notification(
    notifications: &mut mpsc::UnboundedReceiver<OptimisticPipelineNotification>,
) -> OptimisticPipelineNotification {
    tokio::time::timeout(Duration::from_secs(10), notifications.recv())
        .await
        .expect("optimistic pipeline notification timed out")
        .expect("optimistic pipeline notification channel closed")
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn next_block_executes_while_previous_comparator_is_gated(
    #[future]
    #[with(Duration::from_secs(3_000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;
    let mempool = devnet_setup.mempool.clone();
    let _mempool_task = AbortOnDrop::spawn(async move {
        mempool.run_mempool_task(ServiceContext::new_for_testing()).await.expect("mempool service should run")
    });
    tokio::task::yield_now().await;
    let executor = devnet_setup.contracts.0[0].address;
    let first_amount = Felt::from(41u64);
    let second_amount = Felt::from(42u64);
    let last_amount_key = get_storage_var_address("last_amount", &[]).to_felt();
    let first_tx = submit_rust_exec_transfer(&devnet_setup, Felt::ZERO, "transfer", first_amount).await;

    let mut task = devnet_setup
        .block_prod_task()
        .with_startup_execution_mode(crate::fallback::types::StartupExecutionMode::Mixed)
        .with_rust_exec_executor_addresses([executor]);
    let control = task.handle();
    let comparator_gate = task.gate_comparator_for_block(1);
    let mut notifications = task.subscribe_optimistic_pipeline_notifications();
    let _task = AbortOnDrop::spawn(async move {
        task.run(ServiceContext::new_for_testing()).await.expect("block production should run")
    });

    loop {
        if recv_pipeline_notification(&mut notifications).await
            == (OptimisticPipelineNotification::BatchExecuted { block_n: 1 })
        {
            control.close_block().await.expect("block #1 close command should succeed");
            break;
        }
    }

    while recv_pipeline_notification(&mut notifications).await
        != (OptimisticPipelineNotification::ComparatorStarted { block_n: 1 })
    {}

    let second_tx = submit_rust_exec_transfer(&devnet_setup, Felt::ONE, "transfer", second_amount).await;

    loop {
        match recv_pipeline_notification(&mut notifications).await {
            OptimisticPipelineNotification::BatchExecuted { block_n: 2 } => break,
            OptimisticPipelineNotification::ComparatorFinished { block_n: 1 } => {
                panic!("comparator #1 finished before block #2 executed")
            }
            _ => {}
        }
    }

    control.close_block().await.expect("block #2 close command should succeed");
    comparator_gate.add_permits(1);
    loop {
        if recv_pipeline_notification(&mut notifications).await
            == (OptimisticPipelineNotification::ComparatorFinished { block_n: 1 })
        {
            break;
        }
    }

    wait_for_confirmed_block(&devnet_setup.backend, 2).await;
    assert!(block_contains_tx(&devnet_setup.backend, 1, first_tx));
    assert!(block_contains_tx(&devnet_setup.backend, 2, second_tx));
    assert_eq!(block_storage_value(&devnet_setup.backend, 2, last_amount_key), second_amount);
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn n_minus_10_gate_bounds_optimistic_window(
    #[future]
    #[with(Duration::from_millis(20), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;
    let mut task =
        devnet_setup.block_prod_task().with_startup_execution_mode(crate::fallback::types::StartupExecutionMode::Mixed);
    let comparator_gate = task.gate_comparator_for_block(1);
    let mut notifications = task.subscribe_optimistic_pipeline_notifications();
    let _task = AbortOnDrop::spawn(async move {
        task.run(ServiceContext::new_for_testing()).await.expect("block production should run")
    });

    let mut started = HashSet::new();
    while !started.contains(&10) {
        if let OptimisticPipelineNotification::BlockStarted { block_n } =
            recv_pipeline_notification(&mut notifications).await
        {
            assert!(block_n <= 10, "block #{block_n} started beyond the N-10 speculative window");
            started.insert(block_n);
        }
    }
    assert_eq!(started, (1..=10).collect());

    let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    while let Ok(Some(notification)) = tokio::time::timeout_at(deadline, notifications.recv()).await {
        if let OptimisticPipelineNotification::BlockStarted { block_n } = notification {
            assert_ne!(block_n, 11, "block #11 must wait for confirmed block #1");
        }
    }

    comparator_gate.add_permits(1);
    loop {
        if recv_pipeline_notification(&mut notifications).await
            == (OptimisticPipelineNotification::BlockStarted { block_n: 11 })
        {
            break;
        }
    }
}

#[rstest::rstest]
#[timeout(Duration::from_secs(45))]
#[tokio::test]
async fn real_comparator_mismatch_replays_descendants_and_sticks_to_blockifier_only(
    #[future]
    #[with(Duration::from_secs(3_000), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;
    let mempool = devnet_setup.mempool.clone();
    let _mempool_task = AbortOnDrop::spawn(async move {
        mempool.run_mempool_task(ServiceContext::new_for_testing()).await.expect("mempool service should run")
    });
    tokio::task::yield_now().await;
    let executor = devnet_setup.contracts.0[0].address;
    let anchor_amount = Felt::from(11u64);
    let descendant_amount = Felt::from(22u64);
    let post_fallback_amount = Felt::from(33u64);
    let last_amount_key = get_storage_var_address("last_amount", &[]).to_felt();

    let anchor_tx =
        submit_rust_exec_transfer(&devnet_setup, Felt::ZERO, "transfer_with_comparator_mismatch", anchor_amount).await;

    let mut task = devnet_setup
        .block_prod_task()
        .with_startup_execution_mode(crate::fallback::types::StartupExecutionMode::Mixed)
        .with_rust_exec_executor_addresses([executor]);
    let control = task.handle();
    let comparator_gate = task.gate_comparator_for_block(1);
    let mut notifications = task.subscribe_optimistic_pipeline_notifications();
    let _task = AbortOnDrop::spawn(async move {
        task.run(ServiceContext::new_for_testing()).await.expect("block production should run")
    });

    while recv_pipeline_notification(&mut notifications).await
        != (OptimisticPipelineNotification::BatchExecuted { block_n: 1 })
    {}
    control.close_block().await.expect("anchor close command should succeed");
    while recv_pipeline_notification(&mut notifications).await
        != (OptimisticPipelineNotification::ComparatorStarted { block_n: 1 })
    {}

    let descendant_tx = submit_rust_exec_transfer(&devnet_setup, Felt::ONE, "transfer", descendant_amount).await;
    loop {
        match recv_pipeline_notification(&mut notifications).await {
            OptimisticPipelineNotification::BatchExecuted { block_n: 2 } => break,
            OptimisticPipelineNotification::ComparatorFinished { block_n: 1 } => {
                panic!("anchor comparator finished before descendant block execution")
            }
            _ => {}
        }
    }
    let speculative_head = devnet_setup.backend.chain_head_state();
    assert_eq!(speculative_head.confirmed_tip, Some(0));
    assert_eq!(speculative_head.internal_preconfirmed_tip, Some(2));

    comparator_gate.add_permits(1);
    while recv_pipeline_notification(&mut notifications).await
        != (OptimisticPipelineNotification::ComparatorFinished { block_n: 1 })
    {}

    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let status = control.executionbox_status().await.expect("execution status should be available");
            if status.mode == crate::fallback::types::ExecutionMode::BlockifierOnly
                && status.reason == Some(crate::fallback::types::FallbackReason::StateDiffMismatch)
                && status.taint_block == Some(1)
                && status.replay_backlog_empty
                && devnet_setup.backend.latest_confirmed_block_n().is_some_and(|tip| tip >= 2)
            {
                assert!(!status.comparator_enabled, "comparator must stay disabled after strict fallback");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("strict fallback should close the anchor and replay its descendant");

    assert_eq!(block_storage_value(&devnet_setup.backend, 1, last_amount_key), anchor_amount);
    assert!(block_contains_tx(&devnet_setup.backend, 1, anchor_tx));
    assert!(block_contains_tx(&devnet_setup.backend, 2, descendant_tx));
    assert_eq!(block_storage_value(&devnet_setup.backend, 2, last_amount_key), descendant_amount);

    while notifications.try_recv().is_ok() {}
    let post_fallback_tx =
        submit_rust_exec_transfer(&devnet_setup, Felt::TWO, "transfer_with_comparator_mismatch", post_fallback_amount)
            .await;
    let post_fallback_block_n = loop {
        match recv_pipeline_notification(&mut notifications).await {
            OptimisticPipelineNotification::BatchExecuted { block_n } => break block_n,
            OptimisticPipelineNotification::ComparatorStarted { block_n } => {
                panic!("comparator unexpectedly restarted for block #{block_n}")
            }
            _ => {}
        }
    };
    control.close_block().await.expect("post-fallback close command should succeed");

    let confirm = wait_for_confirmed_block(&devnet_setup.backend, post_fallback_block_n);
    tokio::pin!(confirm);
    loop {
        tokio::select! {
            () = &mut confirm => break,
            notification = notifications.recv() => {
                if let Some(OptimisticPipelineNotification::ComparatorStarted { block_n }) = notification {
                    panic!("comparator unexpectedly restarted for block #{block_n}");
                }
            }
        }
    }

    assert!(block_contains_tx(&devnet_setup.backend, post_fallback_block_n, post_fallback_tx));
    assert_eq!(
        block_storage_value(&devnet_setup.backend, post_fallback_block_n, last_amount_key),
        post_fallback_amount,
        "Blockifier-only execution must use the Cairo fixture value, not Rust's mismatching value"
    );
    let final_status = control.executionbox_status().await.expect("final execution status should be available");
    assert_eq!(final_status.mode, crate::fallback::types::ExecutionMode::BlockifierOnly);
    assert!(!final_status.comparator_enabled);
}
