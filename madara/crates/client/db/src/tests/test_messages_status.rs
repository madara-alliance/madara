#![cfg(test)]

use crate::MadaraBackend;
use mp_block::{FullBlockWithoutCommitments, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_convert::{Felt, L1TransactionHash};
use mp_receipt::{ExecutionResult, L1HandlerTransactionReceipt, TransactionReceipt};
use mp_transactions::L1HandlerTransaction;

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

/// Verifies that writing an L1-handler transaction into a block fills the `(l1_tx_hash||nonce) -> l2_tx_hash` index
/// when the `nonce -> l1_tx_hash` mapping already exists.
#[test]
fn l1_to_l2_secondary_index_is_filled_on_block_write_when_mapping_exists() {
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
