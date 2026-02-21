#![cfg(test)]

use crate::{
    storage::StorageChainTip, test_utils::add_test_block, test_utils::l1_handler_tx_with_receipt, MadaraBackend,
    MadaraStorageRead,
};
use mp_chain_config::ChainConfig;
use mp_convert::{Felt, L1TransactionHash};
use mp_transactions::L1HandlerTransactionWithFee;
use std::sync::Arc;

#[tokio::test]
async fn revert_cleans_l1_message_state_and_rewinds_sync_tip_from_source_metadata() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

    let block_0_hash = add_test_block(&backend, 0, vec![]);
    let reverted_nonce = 9u64;
    let reverted_tx_hash = Felt::from(999u64);
    let source_l1_block = 120u64;
    let l1_tx_hash = L1TransactionHash([0x42; 32]);

    backend
        .write_l1_txn_hash_by_nonce(reverted_nonce, &l1_tx_hash)
        .expect("Writing nonce->l1_tx_hash mapping should succeed");
    assert!(backend
        .insert_message_to_l2_seen_marker(&l1_tx_hash, reverted_nonce)
        .expect("Writing l1_tx_hash+nonce seen marker should succeed"));

    add_test_block(&backend, 1, vec![l1_handler_tx_with_receipt(reverted_nonce, reverted_tx_hash)]);

    backend
        .write_pending_message_to_l2(&L1HandlerTransactionWithFee::new(
            mp_transactions::L1HandlerTransaction { nonce: reverted_nonce, ..Default::default() },
            1,
        ))
        .expect("Writing pending message should succeed");
    backend
        .write_l1_handler_l1_block_by_nonce(reverted_nonce, source_l1_block)
        .expect("Writing source L1 block mapping should succeed");
    backend.write_l1_messaging_sync_tip(Some(10_000)).expect("Writing sync tip should succeed");

    let (new_tip_n, new_tip_hash) =
        backend.revert_to(&block_0_hash).expect("Revert should succeed with source mapping present");

    assert_eq!(new_tip_n, 0);
    assert_eq!(new_tip_hash, block_0_hash);
    assert!(matches!(backend.db.get_chain_tip().expect("DB read should succeed"), StorageChainTip::Confirmed(0)));

    assert!(backend.get_l1_handler_txn_hash_by_nonce(reverted_nonce).expect("DB read should succeed").is_none());
    assert!(backend.get_pending_message_to_l2(reverted_nonce).expect("DB read should succeed").is_none());
    assert!(backend.get_l1_handler_l1_block_by_nonce(reverted_nonce).expect("DB read should succeed").is_none());
    assert!(backend.get_l1_txn_hash_by_nonce(reverted_nonce).expect("DB read should succeed").is_none());
    assert!(backend.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash).expect("DB read should succeed").is_none());
    assert_eq!(backend.get_l1_messaging_sync_tip().expect("DB read should succeed"), Some(source_l1_block - 1));
}
