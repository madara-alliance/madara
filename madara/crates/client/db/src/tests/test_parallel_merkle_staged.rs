#![cfg(test)]

use crate::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use crate::rocksdb::RocksDBConfig;
use crate::storage::{MadaraStorageRead, MadaraStorageWrite, StorageChainTip};
use crate::test_utils::declare_v3;
use crate::{MadaraBackend, MadaraBackendConfig};
use blockifier::bouncer::BouncerWeights;
use itertools::Itertools;
use mp_block::{
    commitments::{BlockCommitments, CommitmentComputationContext},
    header::PreconfirmedHeader,
    FullBlockWithoutCommitments, TransactionWithReceipt,
};
use mp_chain_config::ChainConfig;
use mp_class::ClassInfo;
use mp_convert::{Felt, ToFelt};
use mp_receipt::{Event, EventWithTransactionHash, InvokeTransactionReceipt};
use mp_state_update::{ContractStorageDiffItem, MigratedClassItem, NonceUpdate, StateDiff, StorageEntry};
use mp_transactions::{validated::TxTimestamp, InvokeTransactionV0, L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::rstest;

fn make_staged_block(block_n: u64, tx_hash: Felt) -> FullBlockWithoutCommitments {
    let header = PreconfirmedHeader { block_number: block_n, ..Default::default() };
    let tx = TransactionWithReceipt {
        transaction: InvokeTransactionV0::default().into(),
        receipt: InvokeTransactionReceipt { transaction_hash: tx_hash, ..Default::default() }.into(),
    };
    let events = vec![EventWithTransactionHash {
        transaction_hash: tx_hash,
        event: Event {
            from_address: Felt::from(123_u64),
            keys: vec![Felt::from(1_u64)],
            data: vec![Felt::from(2_u64)],
        },
    }];

    FullBlockWithoutCommitments { header, state_diff: StateDiff::default(), transactions: vec![tx], events }
}

fn apply_events_to_transactions_for_test(
    transactions: &[TransactionWithReceipt],
    events: &[EventWithTransactionHash],
) -> Vec<TransactionWithReceipt> {
    let mut events = events.iter().peekable();
    transactions
        .iter()
        .cloned()
        .map(|mut tx| {
            let tx_hash = *tx.receipt.transaction_hash();
            tx.receipt.events_mut().clear();
            tx.receipt.events_mut().extend(
                events.peeking_take_while(|event| event.transaction_hash == tx_hash).map(|event| event.event.clone()),
            );
            tx
        })
        .collect()
}

fn make_preconfirmed_executed_tx(tx_hash: Felt) -> PreconfirmedExecutedTransaction {
    PreconfirmedExecutedTransaction {
        transaction: TransactionWithReceipt {
            transaction: InvokeTransactionV0::default().into(),
            receipt: InvokeTransactionReceipt { transaction_hash: tx_hash, ..Default::default() }.into(),
        },
        state_diff: Default::default(),
        declared_class: None,
        arrived_at: TxTimestamp::now(),
        paid_fee_on_l1: None,
    }
}

fn make_pending_l1_to_l2_message(core_contract_nonce: u64) -> L1HandlerTransactionWithFee {
    L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            nonce: core_contract_nonce,
            contract_address: Felt::from_hex_unchecked("0x1234"),
            entry_point_selector: Felt::from_hex_unchecked("0x99"),
            ..Default::default()
        },
        10,
    )
}

fn stage_blocks(backend: &std::sync::Arc<MadaraBackend>, block_numbers: &[u64], tx_hash_base: u64) {
    for block_n in block_numbers {
        backend
            .write_access()
            .write_parallel_merkle_staged_block_data(
                &make_staged_block(*block_n, Felt::from(tx_hash_base + block_n)),
                &[],
                None,
            )
            .expect("staged write should succeed");
    }
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_persists_without_tip_advance() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0xabc"));

    backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, &[], None)
        .expect("staged write should succeed");

    assert_eq!(backend.latest_confirmed_block_n(), None, "staged write must not advance confirmed tip");
    assert!(backend.has_parallel_merkle_staged_block(0).unwrap(), "staged marker should be present");
    assert_eq!(backend.get_parallel_merkle_staged_blocks().unwrap(), vec![0]);
    assert!(backend.block_view_on_confirmed(0).is_none(), "header mapping must not exist before confirm");
    let staged_info = backend.db.get_block_info(0).unwrap().expect("staged block info should be persisted");
    assert_eq!(
        staged_info.tx_hashes,
        vec![*block.transactions[0].receipt.transaction_hash()],
        "staged block info should contain tx hashes",
    );
    assert_eq!(staged_info.header.block_number, 0);
    assert_eq!(staged_info.header.transaction_count, 1);
    assert_eq!(staged_info.header.event_count, 1);
    assert_eq!(staged_info.header.state_diff_length, Some(block.state_diff.len() as u64));
    assert!(staged_info.header.state_diff_commitment.is_some());
    assert!(staged_info.header.receipt_commitment.is_some());
    assert_eq!(staged_info.block_hash, Felt::ZERO);
    assert!(backend.db.get_block_state_diff(0).unwrap().is_some(), "state diff should be persisted");
    assert_eq!(
        backend.db.get_parallel_merkle_staged_block_header(0).unwrap(),
        Some(block.header.clone()),
        "staged preconfirmed header should be persisted"
    );
}

#[rstest]
#[case::stage1(&[0], Some(0), (0, 10, Some(0)), None)]
#[case::gap(&[0, 2], Some(0), (0, 10, Some(0)), Some((2, 10, Some(2))))]
#[case::cap_confirmed_plus_ten(
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
    Some(10),
    (0, 10, Some(10)),
    None
)]
#[tokio::test]
async fn chain_head_state_stage1_contiguous_and_cap_rules(
    #[case] staged_blocks: &'static [u64],
    #[case] expected_internal_tip: Option<u64>,
    #[case] contiguous_query: (u64, u64, Option<u64>),
    #[case] second_contiguous_query: Option<(u64, u64, Option<u64>)>,
) {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, None);

    stage_blocks(&backend, staged_blocks, 0xabc100_u64);

    let head_state = backend.chain_head_state();
    assert_eq!(head_state.confirmed_tip, None);
    assert_eq!(head_state.external_preconfirmed_tip, None);
    assert_eq!(head_state.internal_preconfirmed_tip, expected_internal_tip);

    let (from_1, to_1, expected_1) = contiguous_query;
    assert_eq!(backend.get_parallel_merkle_staged_contiguous_tip(from_1, to_1).unwrap(), expected_1);

    if let Some((from_2, to_2, expected_2)) = second_contiguous_query {
        assert_eq!(backend.get_parallel_merkle_staged_contiguous_tip(from_2, to_2).unwrap(), expected_2);
    }
}

#[tokio::test]
async fn chain_head_state_tracks_external_and_internal_preconfirmed() {
    let backend = MadaraBackend::open_for_testing_with_config(
        ChainConfig::madara_test().into(),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    let confirmed_block = make_staged_block(0, Felt::from_hex_unchecked("0xabc200"));
    backend
        .write_access()
        .add_full_block_with_classes(&confirmed_block, &[], /* pre_v0_13_2_hash_override */ true)
        .expect("confirmed write should succeed");

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new preconfirmed block should succeed");

    for block_n in 1..=3_u64 {
        backend
            .write_access()
            .write_parallel_merkle_staged_block_data(
                &make_staged_block(block_n, Felt::from(0xabc210_u64 + block_n)),
                &[],
                None,
            )
            .expect("staged write should succeed");
    }

    let head_state = backend.chain_head_state();
    assert_eq!(head_state.confirmed_tip, Some(0));
    assert_eq!(head_state.external_preconfirmed_tip, Some(1));
    assert_eq!(head_state.internal_preconfirmed_tip, Some(3));
}

#[tokio::test]
async fn chain_head_state_reconstructs_from_db_on_startup() {
    let temp_dir = tempfile::TempDir::with_prefix("madara-chain-head").expect("temp dir should be created");
    let chain_config: std::sync::Arc<ChainConfig> = ChainConfig::madara_test().into();
    let builder = mc_class_exec::config::NativeConfig::builder();
    mc_class_exec::init_compilation_semaphore(builder.max_concurrent_compilations());
    let cairo_native_config = std::sync::Arc::new(builder.build());

    let backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config.clone(),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        RocksDBConfig::default(),
        cairo_native_config.clone(),
    )
    .expect("backend should open");

    let confirmed_block = make_staged_block(0, Felt::from_hex_unchecked("0xabc300"));
    backend
        .write_access()
        .add_full_block_with_classes(&confirmed_block, &[], /* pre_v0_13_2_hash_override */ true)
        .expect("confirmed write should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("new preconfirmed block should succeed");
    for block_n in 1..=3_u64 {
        backend
            .write_access()
            .write_parallel_merkle_staged_block_data(
                &make_staged_block(block_n, Felt::from(0xabc310_u64 + block_n)),
                &[],
                None,
            )
            .expect("staged write should succeed");
    }
    drop(backend);

    let restarted_backend = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        RocksDBConfig::default(),
        cairo_native_config,
    )
    .expect("backend should reopen");
    let head_state = restarted_backend.chain_head_state();

    assert_eq!(head_state.confirmed_tip, Some(0));
    assert_eq!(head_state.external_preconfirmed_tip, Some(1));
    assert_eq!(head_state.internal_preconfirmed_tip, Some(3));
}

#[rstest]
#[case::duplicate_after_stage("0x1", "already exists", false)]
#[case::already_confirmed("0x55", "already confirmed", true)]
#[tokio::test]
async fn write_parallel_merkle_staged_block_data_rejects_invalid_stage_target(
    #[case] tx_hash_hex: &str,
    #[case] expected_error: &str,
    #[case] confirm_first: bool,
) {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked(tx_hash_hex));

    if confirm_first {
        backend
            .write_access()
            .add_full_block_with_classes(&block, &[], /* pre_v0_13_2_hash_override */ true)
            .expect("block should confirm");
    } else {
        backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    }

    let err = backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, &[], None)
        .expect_err("invalid staged target must fail");

    let err_text = format!("{err:#}");
    assert!(err_text.contains(expected_error), "unexpected error: {err_text}");
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_stage_one_is_atomic_with_nonce_removals() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let consumed_nonce = 77_u64;
    let pending = make_pending_l1_to_l2_message(consumed_nonce);
    backend.write_pending_message_to_l2(&pending).expect("pending message should be stored");

    // Use an overflowed block number so stage-1 fails before batch commit.
    let block = make_staged_block(u64::MAX, Felt::from_hex_unchecked("0xdead"));
    let err = backend
        .write_access()
        .write_parallel_merkle_staged_block_data_with_consumed_nonces(&block, &[], None, [consumed_nonce])
        .expect_err("staged write should fail for overflowed block number");
    assert!(format!("{err:#}").contains("Converting block_n to u32"));

    // Stage-1 is atomic: nonce removal and staged metadata must not be committed on failure.
    assert_eq!(
        backend.get_pending_message_to_l2(consumed_nonce).unwrap(),
        Some(pending),
        "nonce removal must rollback when stage-1 fails"
    );
    assert!(
        !backend.has_parallel_merkle_staged_block(block.header.block_number).unwrap(),
        "staged marker must not be written on stage-1 failure"
    );
    assert_eq!(
        backend.get_parallel_merkle_staged_block_header(block.header.block_number).unwrap(),
        None,
        "staged header must not be written on stage-1 failure"
    );
}

#[tokio::test]
async fn confirm_parallel_merkle_staged_block_with_root_confirms_in_order() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x44"));
    let next_block = make_staged_block(1, Felt::from_hex_unchecked("0x45"));
    let root = Felt::from_hex_unchecked("0x123456");

    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    backend.write_access().write_parallel_merkle_staged_block_data(&next_block, &[], None).unwrap();
    let result = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, root, /* pre_v0_13_2_hash_override */ true)
        .expect("confirm should succeed");

    assert_eq!(result.new_state_root, root);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert!(!backend.has_parallel_merkle_staged_block(0).unwrap(), "staged marker should be removed");
    assert!(backend.has_parallel_merkle_staged_block(1).unwrap(), "next block should remain staged");
    assert_eq!(backend.db.get_parallel_merkle_staged_block_header(0).unwrap(), None);
    assert_eq!(backend.db.find_block_hash(&result.block_hash).unwrap(), Some(0));

    let block_info = backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap();
    assert_eq!(block_info.header.global_state_root, root);
    assert_eq!(block_info.block_hash, result.block_hash);

    let head_state = backend.chain_head_state();
    assert_eq!(head_state.confirmed_tip, Some(0));
    assert_eq!(head_state.internal_preconfirmed_tip, Some(1));
}

#[tokio::test]
async fn confirm_parallel_merkle_staged_block_with_root_rejects_out_of_order() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block_0 = make_staged_block(0, Felt::from_hex_unchecked("0x10"));
    let block_1 = make_staged_block(1, Felt::from_hex_unchecked("0x11"));

    backend.write_access().write_parallel_merkle_staged_block_data(&block_0, &[], None).unwrap();
    backend.write_access().write_parallel_merkle_staged_block_data(&block_1, &[], None).unwrap();

    let err = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(1, Felt::from_hex_unchecked("0x9"), true)
        .expect_err("out-of-order confirm must fail");

    let err_text = format!("{err:#}");
    assert!(err_text.contains("expected next confirmable block #0"), "unexpected error: {err_text}");
    assert_eq!(backend.latest_confirmed_block_n(), None);
    assert!(backend.has_parallel_merkle_staged_block(0).unwrap());
    assert!(backend.has_parallel_merkle_staged_block(1).unwrap());
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_materializes_events_into_receipts() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x777"));
    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();

    let tx = backend.db.get_transaction(0, 0).unwrap().expect("staged tx should be present");
    assert_eq!(tx.receipt.events(), &[block.events[0].event.clone()]);
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_persists_bouncer_weights() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x778"));
    let weights = BouncerWeights { n_txs: 7, ..Default::default() };

    backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, &[], Some(&weights))
        .expect("staged write should persist bouncer weights");

    assert_eq!(backend.db.get_block_bouncer_weights(0).unwrap(), Some(weights));
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_commitments_match_direct_compute() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x779"));
    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();

    let txs_with_events = apply_events_to_transactions_for_test(&block.transactions, &block.events);
    let expected = BlockCommitments::compute(
        &CommitmentComputationContext {
            protocol_version: block.header.protocol_version,
            chain_id: backend.chain_config().chain_id.to_felt(),
        },
        &txs_with_events,
        &block.state_diff,
        &block.events,
    );
    let staged_info = backend.db.get_block_info(0).unwrap().expect("staged block info should exist");

    assert_eq!(staged_info.header.transaction_commitment, expected.transaction.transaction_commitment);
    assert_eq!(staged_info.header.transaction_count, expected.transaction.transaction_count);
    assert_eq!(staged_info.header.event_commitment, expected.event.events_commitment);
    assert_eq!(staged_info.header.event_count, expected.event.events_count);
    assert_eq!(staged_info.header.receipt_commitment, Some(expected.transaction.receipt_commitment));
    assert_eq!(staged_info.header.state_diff_commitment, Some(expected.state_diff.state_diff_commitment));
    assert_eq!(staged_info.header.state_diff_length, Some(expected.state_diff.state_diff_length));
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_applies_state_diff_to_db_reads() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let mut block = make_staged_block(0, Felt::from_hex_unchecked("0x780"));
    let contract_address = Felt::from_hex_unchecked("0x111");
    let storage_key = Felt::from_hex_unchecked("0x222");
    let storage_value = Felt::from_hex_unchecked("0x333");
    let nonce = Felt::from(9_u64);
    block.state_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: storage_key, value: storage_value }],
        }],
        nonces: vec![NonceUpdate { contract_address, nonce }],
        ..Default::default()
    };

    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();

    assert_eq!(backend.db.get_storage_at(0, &contract_address, &storage_key).unwrap(), Some(storage_value));
    assert_eq!(backend.db.get_contract_nonce_at(0, &contract_address).unwrap(), Some(nonce));
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_updates_migrated_class_hash_and_preserves_class() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let (_tx, converted_class) = declare_v3(Felt::from(42_u64), Felt::ZERO);
    let class_hash = *converted_class.class_hash();
    backend.db.write_classes(0, std::slice::from_ref(&converted_class)).expect("pre-store class should succeed");

    let new_v2_hash = Felt::from_hex_unchecked("0x12345");
    let mut block = make_staged_block(0, Felt::from_hex_unchecked("0x781"));
    block.state_diff.migrated_compiled_classes =
        vec![MigratedClassItem { class_hash, compiled_class_hash: new_v2_hash }];

    backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, std::slice::from_ref(&converted_class), None)
        .unwrap();

    let class = backend.db.get_class(&class_hash).unwrap().expect("class must stay present");
    match class.class_info {
        ClassInfo::Sierra(info) => assert_eq!(info.compiled_class_hash_v2, Some(new_v2_hash)),
        ClassInfo::Legacy(_) => panic!("expected a Sierra class"),
    }
}

#[tokio::test]
async fn parallel_merkle_checkpoint_metadata_roundtrip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    backend.write_parallel_merkle_checkpoint(5).unwrap();
    backend.write_parallel_merkle_checkpoint(8).unwrap();

    assert!(backend.has_parallel_merkle_checkpoint(5).unwrap());
    assert!(backend.has_parallel_merkle_checkpoint(8).unwrap());
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().unwrap(), Some(8));
}

#[tokio::test]
async fn preconfirmed_chain_tip_reads_only_current_block_prefix() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let header = PreconfirmedHeader { block_number: 5, ..Default::default() };
    let tx5 = make_preconfirmed_executed_tx(Felt::from_hex_unchecked("0x5001"));
    let tx6 = make_preconfirmed_executed_tx(Felt::from_hex_unchecked("0x6001"));

    backend
        .db
        .replace_chain_tip(&StorageChainTip::Preconfirmed { header: header.clone(), content: vec![tx5.clone()] })
        .unwrap();
    backend.db.append_preconfirmed_content(6, 0, &[tx6]).unwrap();

    let tip = backend.db.get_chain_tip().unwrap();
    match tip {
        StorageChainTip::Preconfirmed { header: read_header, content } => {
            assert_eq!(read_header.block_number, 5);
            assert_eq!(content.len(), 1, "should only load txs for the current preconfirmed block");
            assert_eq!(content[0].transaction.receipt.transaction_hash(), tx5.transaction.receipt.transaction_hash());
        }
        _ => panic!("expected preconfirmed chain tip"),
    }
}

#[rstest]
#[case::latest_view(None, Some(0), Some(1))]
#[case::boundary_view(Some(0), Some(0), None)]
#[tokio::test]
async fn find_block_by_hash_scopes_results_by_view(
    #[case] view_on_block_n: Option<u64>,
    #[case] expected_block_0: Option<u64>,
    #[case] expected_block_1: Option<u64>,
) {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block_0 = make_staged_block(0, Felt::from_hex_unchecked("0x200"));
    let block_1 = make_staged_block(1, Felt::from_hex_unchecked("0x201"));

    backend.write_access().write_parallel_merkle_staged_block_data(&block_0, &[], None).unwrap();
    let result_0 = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, Felt::from_hex_unchecked("0x2000"), true)
        .unwrap();
    backend.write_access().write_parallel_merkle_staged_block_data(&block_1, &[], None).unwrap();
    let result_1 = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(1, Felt::from_hex_unchecked("0x2001"), true)
        .unwrap();

    let view = match view_on_block_n {
        Some(block_n) => backend.view_on_confirmed(block_n).expect("block should be visible"),
        None => backend.view_on_latest_confirmed(),
    };

    assert_eq!(view.find_block_by_hash(&result_0.block_hash).unwrap(), expected_block_0);
    assert_eq!(view.find_block_by_hash(&result_1.block_hash).unwrap(), expected_block_1);
}

#[tokio::test]
async fn staged_tx_hash_visibility_is_hidden_until_confirmation() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let tx_hash = Felt::from_hex_unchecked("0x9999");
    let block = make_staged_block(0, tx_hash);

    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    assert!(
        backend.view_on_latest_confirmed().find_transaction_by_hash(&tx_hash).unwrap().is_none(),
        "staged tx should not be visible in confirmed-only view"
    );

    backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, Felt::from_hex_unchecked("0x3000"), true)
        .unwrap();
    assert!(
        backend.view_on_latest_confirmed().find_transaction_by_hash(&tx_hash).unwrap().is_some(),
        "tx should become visible after confirmation"
    );
}

#[tokio::test]
async fn compute_parallel_merkle_root_from_snapshot_base_rejects_incompatible_snapshot_base() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    for block_n in 0..=2_u64 {
        let block = make_staged_block(block_n, Felt::from(0x800_u64 + block_n));
        backend
            .write_access()
            .add_full_block_with_classes(&block, &[], /* pre_v0_13_2_hash_override */ true)
            .expect("confirmed write should succeed");
    }

    let err = backend
        .write_access()
        .compute_parallel_merkle_root_from_snapshot_base(
            0,
            3,
            &StateDiff::default(),
            false,
            crate::ParallelMerkleInMemoryTrieLogMode::Off,
        )
        .expect_err("older-than-head snapshot base without historical snapshots must fail");

    let err_text = format!("{err:#}");
    assert!(
        err_text.contains("No compatible snapshot at-or-before requested snapshot_base_block_n=0"),
        "unexpected error: {err_text}"
    );
}

#[tokio::test]
async fn revert_to_replays_from_checkpoint_floor_and_lands_on_exact_requested_block() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let mut block_hashes = Vec::new();
    for block_n in 0..=4_u64 {
        let block = make_staged_block(block_n, Felt::from(0xA000_u64 + block_n));
        let result = backend
            .write_access()
            .add_full_block_with_classes(&block, &[], /* pre_v0_13_2_hash_override */ true)
            .expect("confirmed write should succeed");
        block_hashes.push(result.block_hash);
    }

    // Only floor checkpoints are marked to emulate sparse persisted trie commit markers.
    backend.write_parallel_merkle_checkpoint(0).expect("checkpoint #0 should be persisted");
    backend.write_parallel_merkle_checkpoint(4).expect("checkpoint #4 should be persisted");

    let requested_block_n = 2_u64;
    let requested_block_hash = block_hashes[requested_block_n as usize];
    let (actual_block_n, actual_block_hash) =
        backend.revert_to(&requested_block_hash).expect("revert to requested block should succeed");

    assert_eq!(actual_block_n, requested_block_n, "revert should land on exact requested block");
    assert_eq!(actual_block_hash, requested_block_hash, "returned hash should match requested block hash");

    match backend.db.get_chain_tip().expect("chain tip should be readable") {
        StorageChainTip::Confirmed(block_n) => {
            assert_eq!(block_n, requested_block_n, "chain tip should be updated to requested block");
        }
        other => panic!("expected confirmed chain tip at requested block, got {other:?}"),
    }
    assert_eq!(backend.db.get_block_info(3).unwrap(), None, "block #3 should be pruned");
    assert_eq!(backend.db.get_block_info(4).unwrap(), None, "block #4 should be pruned");
    assert!(
        backend.has_parallel_merkle_checkpoint(requested_block_n).unwrap(),
        "exact target should have a persisted checkpoint marker",
    );
    assert_eq!(
        backend.get_parallel_merkle_latest_checkpoint().unwrap(),
        Some(requested_block_n),
        "latest checkpoint should be clamped to exact target after reorg",
    );
    let head_state = backend.chain_head_state();
    assert_eq!(head_state.confirmed_tip, Some(requested_block_n));
    assert_eq!(head_state.internal_preconfirmed_tip, None);
}
