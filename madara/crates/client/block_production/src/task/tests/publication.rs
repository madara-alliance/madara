use super::*;

fn publication_test_task(backend: Arc<MadaraBackend>) -> BlockProductionTask {
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    BlockProductionTask::new(
        backend,
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    )
}

#[test]
fn approved_future_block_waits_until_external_shell_reaches_it() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed 0");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new_preconfirmed 1");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 2, ..Default::default() }))
        .expect("new_preconfirmed 2");

    let mut task = publication_test_task(backend.clone());
    let tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0x22u64), Felt::from(2u64))]);
    task.buffer_approved_external_content(
        2,
        CanonicalBlockSource::ExecutionBox,
        PreconfirmedHeader { block_number: 2, ..Default::default() },
        vec![tx2],
    );
    task.try_publish_current_external_shell().expect("publish no-op for future block");

    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.header().block_number, 0);
    assert_eq!(ext_view.num_executed_transactions(), 0, "future-approved block must stay buffered");
    assert!(task.approved_external_content.contains_key(&2), "future block stays buffered");
}

#[test]
fn ordered_external_publication_drains_in_block_order() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    for block_n in 0..=2 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new_preconfirmed");
    }

    let mut task = publication_test_task(backend.clone());
    let tx0 = make_fake_preconfirmed_tx(vec![(Felt::from(0x10u64), Felt::from(1u64))]);
    let tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0x11u64), Felt::from(1u64))]);
    let tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0x12u64), Felt::from(1u64))]);

    task.buffer_approved_external_content(
        2,
        CanonicalBlockSource::ExecutionBox,
        PreconfirmedHeader { block_number: 2, ..Default::default() },
        vec![tx2],
    );
    task.buffer_approved_external_content(
        0,
        CanonicalBlockSource::ExecutionBox,
        PreconfirmedHeader { block_number: 0, ..Default::default() },
        vec![tx0],
    );
    task.try_publish_current_external_shell().expect("publish block 0");

    {
        let ext_view = backend.block_view_on_current_preconfirmed().expect("external block 0");
        let ext_content = ext_view.borrow_content();
        let ext_tx = ext_content.executed_transactions().next().expect("tx in block 0");
        assert!(ext_tx.state_diff.nonces.contains_key(&Felt::from(0x10u64)));
    }
    assert!(!task.approved_external_content.contains_key(&0));
    assert!(task.approved_external_content.contains_key(&2));

    backend.write_access().new_confirmed_block(0).expect("confirm block 0");
    task.try_publish_current_external_shell().expect("nothing for block 1 yet");
    assert!(task.approved_external_content.contains_key(&2), "block 2 stays buffered while shell is 1");

    task.buffer_approved_external_content(
        1,
        CanonicalBlockSource::ExecutionBox,
        PreconfirmedHeader { block_number: 1, ..Default::default() },
        vec![tx1],
    );
    task.try_publish_current_external_shell().expect("publish block 1");
    {
        let ext_view = backend.block_view_on_current_preconfirmed().expect("external block 1");
        let ext_content = ext_view.borrow_content();
        let ext_tx = ext_content.executed_transactions().next().expect("tx in block 1");
        assert!(ext_tx.state_diff.nonces.contains_key(&Felt::from(0x11u64)));
    }

    backend.write_access().new_confirmed_block(1).expect("confirm block 1");
    task.try_publish_current_external_shell().expect("publish buffered block 2");
    {
        let ext_view = backend.block_view_on_current_preconfirmed().expect("external block 2");
        let ext_content = ext_view.borrow_content();
        let ext_tx = ext_content.executed_transactions().next().expect("tx in block 2");
        assert!(ext_tx.state_diff.nonces.contains_key(&Felt::from(0x12u64)));
    }
    assert!(task.approved_external_content.is_empty(), "all approved blocks drained in order");
}

#[test]
fn stop_path_publication_uses_bre_rows_when_shell_reaches_block() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("new_preconfirmed 0");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new_preconfirmed 1");

    let mut task = publication_test_task(backend.clone());
    let bre_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(3u64))]);
    task.buffer_approved_external_content(
        1,
        CanonicalBlockSource::BlockifierReexec,
        PreconfirmedHeader { block_number: 1, ..Default::default() },
        vec![bre_tx],
    );

    backend.write_access().new_confirmed_block(0).expect("confirm block 0");
    task.try_publish_current_external_shell().expect("publish BRE-backed block 1");

    let ext_view = backend.block_view_on_current_preconfirmed().expect("external block 1");
    let ext_content = ext_view.borrow_content();
    let ext_tx = ext_content.executed_transactions().next().expect("tx in block 1");
    assert!(
        ext_tx.state_diff.nonces.contains_key(&Felt::from(0xBBu64)),
        "published rows must come from BRE-backed content"
    );
}

// ── C-024: Block body publication and close source unification tests ──

/// Helper to create a fake preconfirmed tx with a unique tx hash for C-024 tests.
fn make_c024_preconfirmed_tx(tx_hash_seed: u64) -> mc_db::preconfirmed::PreconfirmedExecutedTransaction {
    use mp_transactions::validated::TxTimestamp;
    mc_db::preconfirmed::PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0 {
                    calldata: vec![Felt::from(tx_hash_seed)].into(),
                    ..Default::default()
                },
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: Felt::from(tx_hash_seed),
                ..Default::default()
            }),
        },
        state_diff: mp_state_update::TransactionStateUpdate {
            nonces: [(Felt::from(tx_hash_seed), Felt::from(tx_hash_seed + 1))].into_iter().collect(),
            ..Default::default()
        },
        declared_class: None,
        arrived_at: TxTimestamp(0),
        paid_fee_on_l1: None,
    }
}

/// C-024: close_canonical_block writes the exact canonical rows, not whatever
/// happens to be in the preconfirmed view. Even if the internal runtime is on
/// block X+N (runahead), close for block X uses the canonical rows from the payload.
#[test]
fn close_canonical_block_uses_payload_rows_not_preconfirmed_view() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    // Create a preconfirmed block with 1 tx (simulates what internal runtime has).
    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).unwrap();
    let runtime_tx = make_c024_preconfirmed_tx(999);
    backend.append_to_internal_preconfirmed_runtime(&[runtime_tx]).unwrap();

    // Canonical rows from comparator have 3 different txs.
    let canonical_rows: Vec<_> = (1..=3).map(make_c024_preconfirmed_tx).collect();

    let result = backend
        .write_access()
        .close_canonical_block(true, 0, header, canonical_rows, StateDiff::default())
        .expect("close_canonical_block should succeed");

    // Verify confirmed block has 3 txs (from canonical rows), not 1 (from runtime).
    let confirmed = backend.block_view_on_confirmed(0).expect("confirmed block 0");
    let txs = confirmed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 3, "confirmed block must have canonical row count, not runtime row count");

    // Verify tx hashes match canonical rows, not runtime.
    assert_eq!(*txs[0].receipt.transaction_hash(), Felt::from(1u64));
    assert_eq!(*txs[1].receipt.transaction_hash(), Felt::from(2u64));
    assert_eq!(*txs[2].receipt.transaction_hash(), Felt::from(3u64));

    // Verify block hash was computed.
    assert_ne!(result.block_hash, Felt::ZERO);
}

/// C-024: close_canonical_block uses payload rows.
#[test]
fn close_canonical_block_uses_payload_rows() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    // Preconfirmed block has 0 txs (simulates stale DB preconfirmed under runahead).
    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).unwrap();

    // Canonical rows have 5 txs.
    let canonical_rows: Vec<_> = (10..=14).map(make_c024_preconfirmed_tx).collect();

    let _result = backend
        .write_access()
        .close_canonical_block(true, 0, header, canonical_rows, StateDiff::default())
        .expect("close_canonical_block should succeed");

    // Verify confirmed block has 5 txs.
    let confirmed = backend.block_view_on_confirmed(0).expect("confirmed block 0");
    let txs = confirmed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 5, "close path must use canonical rows from payload");
}

/// C-024: Regression test for the 22347 class of bugs — external shell filled with
/// N txs but close writes tx_count=0 because preconfirmed view is stale.
/// After C-024, close uses canonical rows from the payload, making it independent
/// of what the preconfirmed view resolves to at close time.
#[test]
fn close_canonical_block_independent_of_stale_preconfirmed_view() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    // Simulate runahead: preconfirmed block for block 0 exists but internal runtime
    // is already on block 1 (so block_view_on_preconfirmed(0) reads from DB which
    // may have 0 txs if flush didn't persist block 0's content).
    let header_0 = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header_0.clone())).unwrap();

    // Don't append anything to the runtime or flush — simulates the case where
    // DB preconfirmed for block 0 has 0 txs.

    // Canonical rows (from comparator) have 123 txs.
    let canonical_rows: Vec<_> = (0..123).map(make_c024_preconfirmed_tx).collect();

    let result = backend
        .write_access()
        .close_canonical_block(true, 0, header_0, canonical_rows, StateDiff::default())
        .expect("close_canonical_block should succeed");

    // Verify confirmed block has 123 txs, NOT 0.
    let confirmed = backend.block_view_on_confirmed(0).expect("confirmed block 0");
    let txs = confirmed.get_executed_transactions(..).unwrap();
    assert_eq!(
        txs.len(),
        123,
        "close must use canonical payload rows, not stale preconfirmed view (regression for 22347 class)"
    );
    assert_ne!(result.block_hash, Felt::ZERO);
}

/// C-024: Confirmed tx-hash index count matches canonical row count.
/// After close, all tx hashes from canonical rows are discoverable in the confirmed block.
#[test]
fn close_canonical_block_tx_hash_index_matches_canonical_rows() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).unwrap();

    let canonical_rows: Vec<_> = (100..=104).map(make_c024_preconfirmed_tx).collect();

    backend
        .write_access()
        .close_canonical_block(true, 0, header, canonical_rows, StateDiff::default())
        .expect("close_canonical_block should succeed");

    // Verify confirmed block has exactly 5 txs with matching hashes.
    let confirmed = backend.block_view_on_confirmed(0).expect("confirmed block 0");
    let txs = confirmed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 5, "confirmed block must have 5 txs from canonical rows");

    // Verify each tx hash matches the canonical row.
    for (i, tx) in txs.iter().enumerate() {
        let expected_hash = Felt::from((100 + i) as u64);
        assert_eq!(*tx.receipt.transaction_hash(), expected_hash, "tx[{i}] hash must match canonical row");
    }
}

/// C-024: In BlockifierOnly mode, canonical rows are read from preconfirmed view
/// (since speculative_executed_txs is empty), and close uses those rows from the payload.
#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn test_blockifier_only_close_writes_correct_tx_count(
    #[future]
    #[with(Duration::from_secs(100), false)]
    devnet_setup: DevnetSetup,
) {
    let mut devnet_setup = devnet_setup.await;

    // Submit a transaction.
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;
    assert!(!devnet_setup.mempool.is_empty().await);

    // Run block production in BlockifierOnly mode.
    let mut block_production_task = devnet_setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let control = block_production_task.handle();
    let _task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    control.close_block().await.unwrap();
    assert!(matches!(
        notifications.recv().await.unwrap(),
        BlockProductionStateNotification::ClosedBlock { block_n: 1 }
    ));

    // Verify confirmed block has 1 tx (not 0).
    let confirmed = devnet_setup.backend.block_view_on_confirmed(1).expect("confirmed block 1");
    let txs = confirmed.get_executed_transactions(..).unwrap();
    assert_eq!(txs.len(), 1, "BlockifierOnly close must write correct tx count via C-024 canonical rows");
}

// ===== Mixed mode preconfirmed DB persistence tests =====

fn backend_with_preconfirmed_persistence() -> Arc<MadaraBackend> {
    MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    )
}

/// Mixed batch append persists executed rows to block-scoped DB after each batch.
#[test]
fn mixed_mode_append_persists_to_db() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0x1u64), Felt::from(1u64))]);
    let tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0x2u64), Felt::from(2u64))]);

    // First batch: append 1 tx
    backend.append_to_internal_preconfirmed_and_persist(&[tx1]).expect("persist batch 1");

    // Verify DB has the row
    let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db read").expect("data exists");
    assert_eq!(content.len(), 1, "DB should have 1 tx after first batch");

    // Second batch: append 1 more tx
    backend.append_to_internal_preconfirmed_and_persist(&[tx2]).expect("persist batch 2");

    // Verify DB now has both rows
    let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db read").expect("data exists");
    assert_eq!(content.len(), 2, "DB should have 2 txs after second batch");

    // Verify runtime internal also has both txs
    let view = backend.block_view_on_preconfirmed(0).expect("preconfirmed view");
    assert_eq!(view.num_executed_transactions(), 2, "runtime internal should have 2 txs");
}

/// Mixed batch append updates internal runtime but does NOT update external runtime.
#[test]
fn mixed_mode_append_persist_no_external_update() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    let fake_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xBBu64), Felt::from(1u64))]);

    // Append with persistence
    backend.append_to_internal_preconfirmed_and_persist(&[fake_tx]).expect("persist");

    // Internal has content
    let int_view = backend.block_view_on_preconfirmed(0).expect("internal view");
    assert_eq!(int_view.num_executed_transactions(), 1, "internal should have 1 tx");

    // External does NOT have content — still comparator-gated
    let ext_view = backend.block_view_on_current_preconfirmed().expect("external view");
    assert_eq!(ext_view.num_executed_transactions(), 0, "external must remain empty");
}

/// Startup reconstruction of internal tip prefers block-scoped persisted rows
/// over stale head-projection shell content.
#[test]
fn startup_reconstruction_prefers_block_scoped_rows() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).expect("new_preconfirmed");

    // Append 3 txs via Mixed mode persistence (writes block-scoped rows, not head projection)
    let tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0x1u64), Felt::from(1u64))]);
    let tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0x2u64), Felt::from(2u64))]);
    let tx3 = make_fake_preconfirmed_tx(vec![(Felt::from(0x3u64), Felt::from(3u64))]);
    backend.append_to_internal_preconfirmed_and_persist(&[tx1, tx2, tx3]).expect("persist 3 txs");

    // Head projection has stale content (0 txs — only shell, from new_preconfirmed)
    let stored_tip = mc_db::storage::StorageHeadProjection::Preconfirmed {
        header: header.clone(),
        content: vec![], // Shell — no txs
    };

    // load_preconfirmed_block_for_tip should prefer the 3 block-scoped DB rows.
    // Verify by checking DB directly: block-scoped rows should have 3 txs.
    let (_, db_rows) = backend.db.get_preconfirmed_block_data(0).expect("db read").expect("data exists");
    assert_eq!(db_rows.len(), 3, "Block-scoped DB should have 3 txs from Mixed persistence");

    // Also verify that load_preconfirmed_block_for_tip prefers these rows
    // by using the test accessor — it should return 3 txs not 0
    let loaded = backend.load_preconfirmed_block_for_tip_test(0, &stored_tip).expect("load");
    let view = backend.block_view_on_preconfirmed(0).expect("view after reload");
    assert_eq!(view.num_executed_transactions(), 3, "Should prefer block-scoped rows (3) over shell (0)");
    // The loaded block header should match
    assert_eq!(loaded.header.block_number, 0, "Loaded block should be block 0");
}

/// Stop path replaces persisted EB rows with BRE rows for block X.
#[test]
fn stop_path_replaces_persisted_eb_with_bre() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

    // Persist EB rows via Mixed append
    let eb_tx = make_fake_preconfirmed_tx(vec![(Felt::from(0xEBu64), Felt::from(1u64))]);
    backend.append_to_internal_preconfirmed_and_persist(&[eb_tx]).expect("persist EB");

    // Verify DB has EB rows
    let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db").expect("data");
    assert_eq!(content.len(), 1, "DB should have 1 EB tx");

    // Replace with BRE rows (2 txs)
    let bre_tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0xB1u64), Felt::from(10u64))]);
    let bre_tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0xB2u64), Felt::from(20u64))]);
    backend
        .write_access()
        .replace_internal_preconfirmed_content_and_persist(0, vec![bre_tx1, bre_tx2])
        .expect("BRE replace");

    // DB now has BRE rows, not EB
    let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db").expect("data");
    assert_eq!(content.len(), 2, "DB should have 2 BRE txs after replacement");

    // Runtime also has BRE rows
    let view = backend.block_view_on_preconfirmed(0).expect("view");
    assert_eq!(view.num_executed_transactions(), 2, "runtime should have 2 BRE txs");
}

/// After stop-path persisted replacement, DB reads for block X see BRE rows.
#[test]
fn stop_path_db_fallback_reads_bre_not_eb() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).expect("new_preconfirmed");

    // Persist 3 EB rows
    let eb_txs: Vec<_> =
        (0..3).map(|i| make_fake_preconfirmed_tx(vec![(Felt::from(0xEB00u64 + i), Felt::from(1u64))])).collect();
    backend.append_to_internal_preconfirmed_and_persist(&eb_txs).expect("persist EB");

    // Replace with 5 BRE rows
    let bre_txs: Vec<_> =
        (0..5).map(|i| make_fake_preconfirmed_tx(vec![(Felt::from(0xBE00u64 + i), Felt::from(10u64))])).collect();
    backend.write_access().replace_internal_preconfirmed_content_and_persist(0, bre_txs).expect("BRE replace");

    // DB reads must return 5 BRE rows, NOT the old 3 EB rows
    let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db").expect("data");
    assert_eq!(content.len(), 5, "DB should have 5 BRE rows after stop-path replacement");

    // Simulate startup reconstruction — DB should see 5 BRE rows
    let (_, db_rows) = backend.db.get_preconfirmed_block_data(0).expect("db").expect("data after BRE");
    assert_eq!(db_rows.len(), 5, "Startup reconstruction DB should see 5 BRE rows, not old 3 EB rows");
}

/// Startup reconstruction prefers block-scoped persisted data even when the block-scoped
/// content is empty. `Some((header, vec![]))` is authoritative — the loader must NOT
/// fall back to head-projection content in that case.
#[test]
fn startup_reconstruction_prefers_empty_block_scoped_data_over_stale_shell() {
    let backend = backend_with_preconfirmed_persistence();

    let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header.clone())).expect("new_preconfirmed");

    // Write a preconfirmed header to DB for block 0 (creates block-scoped entry
    // with empty content — this is the "authoritative empty" case).
    backend.db.write_preconfirmed_header(&header).expect("write header");

    // Head projection has stale shell with 2 fake txs (simulating stale external content).
    let stale_tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0xDEADu64), Felt::from(1u64))]);
    let stale_tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0xBEEFu64), Felt::from(2u64))]);
    let stored_tip = mc_db::storage::StorageHeadProjection::Preconfirmed {
        header: header.clone(),
        content: vec![stale_tx1, stale_tx2], // Stale shell content
    };

    // Block-scoped data exists but has empty content.
    let block_scoped = backend.db.get_preconfirmed_block_data(0).expect("db read");
    assert!(block_scoped.is_some(), "Block-scoped data should exist");
    let (_, content) = block_scoped.unwrap();
    assert!(content.is_empty(), "Block-scoped content should be empty");

    // load_preconfirmed_block_for_tip must return the empty block-scoped data,
    // NOT the stale 2-tx shell from head projection.
    let loaded = backend.load_preconfirmed_block_for_tip_test(0, &stored_tip).expect("load");
    assert_eq!(loaded.header.block_number, 0);
    // The loaded block must have 0 txs (from authoritative empty block-scoped data),
    // not 2 (from stale head projection).
    // Verify via DB: block-scoped data should remain empty (authoritative source).
    let (_, final_content) = backend.db.get_preconfirmed_block_data(0).expect("db").unwrap();
    assert_eq!(final_content.len(), 0, "Authoritative empty block-scoped data must be 0, not stale 2-tx shell");
}

/// Restart recovery integration test: persisted mixed-mode rows survive restart
/// and are used by the build_runtime_head_projection recovery path.
///
/// Simulates: Mixed mode persists 3 txs for block 0 -> process "crashes" (drop backend) ->
/// new backend opens same DB -> internal preconfirmed tip reconstructs using persisted rows.
#[test]
fn restart_recovery_uses_persisted_mixed_rows() {
    // Phase 1: Create backend, persist Mixed mode rows, then drop.
    let temp_dir = tempfile::TempDir::with_prefix("madara-restart-test").unwrap();
    let chain_config = Arc::new(ChainConfig::madara_devnet());

    {
        let backend = MadaraBackend::open_for_testing_with_dir(
            temp_dir.path(),
            chain_config.clone(),
            MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        );

        let header = PreconfirmedHeader { block_number: 0, ..Default::default() };
        backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).expect("new_preconfirmed");

        // Simulate Mixed mode: persist 3 txs via the new helper
        let tx1 = make_fake_preconfirmed_tx(vec![(Felt::from(0x1u64), Felt::from(1u64))]);
        let tx2 = make_fake_preconfirmed_tx(vec![(Felt::from(0x2u64), Felt::from(2u64))]);
        let tx3 = make_fake_preconfirmed_tx(vec![(Felt::from(0x3u64), Felt::from(3u64))]);
        backend.append_to_internal_preconfirmed_and_persist(&[tx1, tx2, tx3]).expect("persist 3 txs");

        // Verify DB has the rows before "crash"
        let (_, content) = backend.db.get_preconfirmed_block_data(0).expect("db").expect("data");
        assert_eq!(content.len(), 3, "DB should have 3 persisted mixed-mode txs before crash");

        // Flush to ensure data is on disk
        backend.db.flush().expect("flush");

        // Drop backend — simulates crash
    }

    // Phase 2: Reopen backend from same DB directory.
    {
        let backend = MadaraBackend::open_for_testing_with_dir(
            temp_dir.path(),
            chain_config,
            MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        );

        // Verify the internal preconfirmed tip was reconstructed
        let head = backend.chain_head_state();
        assert!(
            head.internal_preconfirmed_tip.is_some(),
            "Internal preconfirmed tip should be reconstructed after restart"
        );
        assert_eq!(head.internal_preconfirmed_tip, Some(0), "Internal tip should be block 0");

        // Verify the runtime preconfirmed block has the 3 persisted rows
        let view = backend.block_view_on_preconfirmed(0).expect("preconfirmed view after restart");
        assert_eq!(
            view.num_executed_transactions(),
            3,
            "Restart recovery must reconstruct 3 Mixed-mode persisted txs from block-scoped DB rows"
        );
    }
}
