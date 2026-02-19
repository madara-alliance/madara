#![cfg(test)]

use crate::preconfirmed::PreconfirmedExecutedTransaction;
use crate::storage::{MadaraStorageRead, MadaraStorageWrite, StorageChainTip};
use crate::test_utils::declare_v3;
use crate::MadaraBackend;
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
use mp_transactions::{validated::TxTimestamp, InvokeTransactionV0};

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

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_rejects_duplicate_block() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x1"));

    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    let err = backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, &[], None)
        .expect_err("duplicate staged write must be rejected");

    let err_text = format!("{err:#}");
    assert!(err_text.contains("already exists"), "unexpected error: {err_text}");
}

#[tokio::test]
async fn write_parallel_merkle_staged_block_data_rejects_confirmed_block() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x55"));
    backend
        .write_access()
        .add_full_block_with_classes(&block, &[], /* pre_v0_13_2_hash_override */ true)
        .expect("block should confirm");

    let err = backend
        .write_access()
        .write_parallel_merkle_staged_block_data(&block, &[], None)
        .expect_err("confirmed block should not be staged again");
    assert!(format!("{err:#}").contains("already confirmed"));
}

#[tokio::test]
async fn confirm_parallel_merkle_staged_block_with_root_confirms_in_order() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0x44"));
    let root = Felt::from_hex_unchecked("0x123456");

    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    let result = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, root, /* pre_v0_13_2_hash_override */ true)
        .expect("confirm should succeed");

    assert_eq!(result.new_state_root, root);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert!(!backend.has_parallel_merkle_staged_block(0).unwrap(), "staged marker should be removed");
    assert_eq!(backend.db.find_block_hash(&result.block_hash).unwrap(), Some(0));

    let block_info = backend.block_view_on_confirmed(0).unwrap().get_block_info().unwrap();
    assert_eq!(block_info.header.global_state_root, root);
    assert_eq!(block_info.block_hash, result.block_hash);
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
async fn confirm_parallel_merkle_staged_block_clears_staged_header_and_state() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = make_staged_block(0, Felt::from_hex_unchecked("0xbeef"));
    backend.write_access().write_parallel_merkle_staged_block_data(&block, &[], None).unwrap();
    backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, Felt::from_hex_unchecked("0xff"), true)
        .unwrap();

    assert!(!backend.has_parallel_merkle_staged_block(0).unwrap());
    assert_eq!(backend.get_parallel_merkle_staged_blocks().unwrap(), Vec::<u64>::new());
    assert_eq!(backend.db.get_parallel_merkle_staged_block_header(0).unwrap(), None);
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

#[tokio::test]
async fn find_block_by_hash_returns_any_confirmed_block_not_only_tip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let block_0 = make_staged_block(0, Felt::from_hex_unchecked("0x100"));
    let block_1 = make_staged_block(1, Felt::from_hex_unchecked("0x101"));

    backend.write_access().write_parallel_merkle_staged_block_data(&block_0, &[], None).unwrap();
    let result_0 = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(0, Felt::from_hex_unchecked("0x1000"), true)
        .unwrap();

    backend.write_access().write_parallel_merkle_staged_block_data(&block_1, &[], None).unwrap();
    let result_1 = backend
        .write_access()
        .confirm_parallel_merkle_staged_block_with_root(1, Felt::from_hex_unchecked("0x1001"), true)
        .unwrap();

    let view = backend.view_on_latest_confirmed();
    assert_eq!(view.find_block_by_hash(&result_0.block_hash).unwrap(), Some(0));
    assert_eq!(view.find_block_by_hash(&result_1.block_hash).unwrap(), Some(1));
}

#[tokio::test]
async fn find_block_by_hash_respects_view_boundary() {
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

    let view_on_block_0 = backend.view_on_confirmed(0).expect("block #0 should be visible");
    assert_eq!(view_on_block_0.find_block_by_hash(&result_0.block_hash).unwrap(), Some(0));
    assert_eq!(view_on_block_0.find_block_by_hash(&result_1.block_hash).unwrap(), None);
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
