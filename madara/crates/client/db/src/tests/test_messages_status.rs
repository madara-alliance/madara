#![cfg(test)]

use crate::{rocksdb::RocksDBConfig, storage::L1ToL2MessageIndexEntry, MadaraBackend, MadaraBackendConfig};
use mc_class_exec::config::NativeConfig;
use mp_block::{BlockHeaderWithSignatures, FullBlockWithoutCommitments, Header, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_convert::{Felt, L1TransactionHash};
use mp_receipt::{ExecutionResult, L1HandlerTransactionReceipt, TransactionReceipt};
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use std::sync::Arc;

/// Verifies the secondary index used by `starknet_getMessagesStatus`:
/// - Unknown L1 tx hash returns `None`
/// - Keys are iterated in nonce order (L1 sending order)
/// - Empty marker values are returned as `None`
/// - Filled values are returned as `Some(l2_tx_hash)` and are not clobbered by later marker inserts
#[test]
fn l1_to_l2_messages_by_l1_tx_hash_roundtrip_and_ordering() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let mut l1_bytes = [0u8; 32];
    l1_bytes[31] = 0x01;
    let l1_tx_hash = L1TransactionHash(l1_bytes);

    // Unknown should be None.
    let unknown = L1TransactionHash([0x11; 32]);
    assert!(db.get_messages_to_l2_by_l1_tx_hash(&unknown).unwrap().is_none());

    // Write out-of-order "seen markers" (empty values).
    assert!(db.insert_message_to_l2_seen_marker(&l1_tx_hash, 10).unwrap());
    assert!(db.insert_message_to_l2_seen_marker(&l1_tx_hash, 9).unwrap());

    let msgs = db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap();
    assert_eq!(msgs.len(), 2);
    // Must be ordered by nonce (L1 sending order).
    assert_eq!(msgs[0], (9, None));
    assert_eq!(msgs[1], (10, None));

    // Fill one consumed tx hash and verify it is returned.
    let l2_tx_hash = Felt::from_hex("0x123").unwrap();
    db.write_message_to_l2_consumed_txn_hash(&l1_tx_hash, 10, &l2_tx_hash).unwrap();

    let msgs = db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap();
    assert_eq!(msgs[0], (9, None));
    assert_eq!(msgs[1], (10, Some(l2_tx_hash)));

    // Ensure does not clobber a filled value.
    assert!(!db.insert_message_to_l2_seen_marker(&l1_tx_hash, 10).unwrap());
    let msgs = db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap();
    assert_eq!(msgs[1], (10, Some(l2_tx_hash)));
}

/// Verifies the `nonce -> l1_tx_hash` mapping roundtrip.
#[test]
fn l1_to_l2_l1_tx_hash_by_nonce_roundtrip() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    assert!(db.get_l1_txn_hash_by_nonce(42).unwrap().is_none());

    let l1_tx_hash = L1TransactionHash([0x22; 32]);
    db.write_l1_txn_hash_by_nonce(42, &l1_tx_hash).unwrap();
    assert_eq!(db.get_l1_txn_hash_by_nonce(42).unwrap(), Some(l1_tx_hash));
}

/// Verifies that confirming an L1-handler transaction fills the `(l1_tx_hash||nonce) -> l2_tx_hash` index
/// when the `nonce -> l1_tx_hash` mapping already exists.
#[test]
fn l1_to_l2_secondary_index_is_filled_on_block_confirmation_when_mapping_exists() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    let nonce = 7u64;
    let l1_tx_hash = L1TransactionHash([0x33; 32]);
    db.write_l1_txn_hash_by_nonce(nonce, &l1_tx_hash).unwrap();
    assert!(db.insert_message_to_l2_seen_marker(&l1_tx_hash, nonce).unwrap());

    let l2_tx_hash = Felt::from_hex("0x123").unwrap();
    let tx = L1HandlerTransaction { nonce, ..Default::default() };
    let receipt = TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
        transaction_hash: l2_tx_hash,
        execution_result: ExecutionResult::Succeeded,
        ..Default::default()
    });
    let block = FullBlockWithoutCommitments {
        header: Default::default(),
        state_diff: Default::default(),
        transactions: vec![TransactionWithReceipt { transaction: tx.into(), receipt }],
        events: Default::default(),
    };

    db.write_access().add_full_block_with_classes(&block, &[], true).unwrap();

    let msgs = db.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).unwrap().unwrap();
    assert_eq!(msgs, vec![(nonce, Some(l2_tx_hash))]);
}

#[test]
fn l1_message_remains_pending_until_block_is_confirmed() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block_n = 0;
    let nonce = 7;
    let l1_tx_hash = L1TransactionHash([0x44; 32]);
    let l2_tx_hash = Felt::from_hex("0x456").unwrap();
    let l1_handler = L1HandlerTransaction { nonce, ..Default::default() };
    let transaction = TransactionWithReceipt {
        transaction: l1_handler.clone().into(),
        receipt: TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            transaction_hash: l2_tx_hash,
            execution_result: ExecutionResult::Succeeded,
            ..Default::default()
        }),
    };

    db.write_l1_txn_hash_by_nonce(nonce, &l1_tx_hash).unwrap();
    assert!(db.insert_message_to_l2_seen_marker(&l1_tx_hash, nonce).unwrap());
    db.write_pending_message_to_l2(&L1HandlerTransactionWithFee::new(l1_handler, 1)).unwrap();

    let writer = db.write_access();
    writer
        .write_header(BlockHeaderWithSignatures {
            header: Header { block_number: block_n, ..Default::default() },
            block_hash: Felt::from(123_u64),
            consensus_signatures: vec![],
        })
        .unwrap();
    writer.write_transactions(block_n, std::slice::from_ref(&transaction)).unwrap();

    assert!(db.get_pending_message_to_l2(nonce).unwrap().is_some());
    assert!(db.get_l1_handler_txn_hash_by_nonce(nonce).unwrap().is_none());
    assert_eq!(db.get_message_to_l2_index_entry(&l1_tx_hash, nonce).unwrap(), Some(L1ToL2MessageIndexEntry::Seen));

    writer.new_confirmed_block(block_n).unwrap();

    assert!(db.get_pending_message_to_l2(nonce).unwrap().is_none());
    assert_eq!(db.get_l1_handler_txn_hash_by_nonce(nonce).unwrap(), Some(l2_tx_hash));
    assert_eq!(
        db.get_message_to_l2_index_entry(&l1_tx_hash, nonce).unwrap(),
        Some(L1ToL2MessageIndexEntry::Consumed(l2_tx_hash))
    );
}

#[test]
fn startup_repairs_l1_projection_after_confirmed_head_advance() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let chain_config = Arc::new(ChainConfig::madara_test());
    let nonce = 11;
    let l1_tx_hash = L1TransactionHash([0x55; 32]);
    let l2_tx_hash = Felt::from_hex("0x789").unwrap();
    let l1_handler = L1HandlerTransaction { nonce, ..Default::default() };
    let pending = L1HandlerTransactionWithFee::new(l1_handler.clone(), 1);

    {
        let db = MadaraBackend::open_rocksdb(
            temp_dir.path(),
            Arc::clone(&chain_config),
            MadaraBackendConfig::default(),
            RocksDBConfig::default(),
            Arc::new(NativeConfig::default()),
        )
        .unwrap();
        db.write_l1_txn_hash_by_nonce(nonce, &l1_tx_hash).unwrap();
        assert!(db.insert_message_to_l2_seen_marker(&l1_tx_hash, nonce).unwrap());
        db.write_pending_message_to_l2(&pending).unwrap();

        let block = FullBlockWithoutCommitments {
            header: mp_block::PreconfirmedHeader { block_number: 0, ..Default::default() },
            state_diff: Default::default(),
            transactions: vec![TransactionWithReceipt {
                transaction: l1_handler.into(),
                receipt: TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
                    transaction_hash: l2_tx_hash,
                    execution_result: ExecutionResult::Succeeded,
                    ..Default::default()
                }),
            }],
            events: Default::default(),
        };
        db.write_access().add_full_block_with_classes(&block, &[], true).unwrap();

        // Simulate a stop after the confirmed head write but before the derived L1 batch.
        db.write_pending_message_to_l2(&pending).unwrap();
        let rocksdb = db.db.inner_db();
        let consumed_cf = rocksdb.cf_handle("l1_to_l2_txn_hash_by_nonce").unwrap();
        rocksdb.delete_cf(&consumed_cf, nonce.to_be_bytes()).unwrap();
        let by_l1_cf = rocksdb.cf_handle("l1_to_l2_l2_txn_hash_by_l1_txn_hash_and_nonce").unwrap();
        let mut by_l1_key = [0_u8; 40];
        by_l1_key[..32].copy_from_slice(&l1_tx_hash.0);
        by_l1_key[32..].copy_from_slice(&nonce.to_be_bytes());
        rocksdb.put_cf(&by_l1_cf, by_l1_key, []).unwrap();
    }

    let reopened = MadaraBackend::open_rocksdb(
        temp_dir.path(),
        chain_config,
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        Arc::new(NativeConfig::default()),
    )
    .unwrap();

    assert_eq!(reopened.latest_confirmed_block_n(), Some(0));
    assert!(reopened.get_pending_message_to_l2(nonce).unwrap().is_none());
    assert_eq!(reopened.get_l1_handler_txn_hash_by_nonce(nonce).unwrap(), Some(l2_tx_hash));
    assert_eq!(
        reopened.get_message_to_l2_index_entry(&l1_tx_hash, nonce).unwrap(),
        Some(L1ToL2MessageIndexEntry::Consumed(l2_tx_hash))
    );
}
