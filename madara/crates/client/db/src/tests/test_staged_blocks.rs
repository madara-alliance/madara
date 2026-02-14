#![cfg(test)]

use crate::{MadaraBackend, MadaraStorageRead};
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_receipt::{Event, EventWithTransactionHash, InvokeTransactionReceipt, TransactionReceipt};
use mp_state_update::{NonceUpdate, StateDiff};
use mp_transactions::InvokeTransactionV0;

fn staged_block(block_number: u64) -> FullBlockWithoutCommitments {
    let tx_hash = Felt::from(block_number + 1000);
    let event = Event {
        from_address: Felt::from(0xabc_u64),
        keys: vec![Felt::from(1_u64)],
        data: vec![Felt::from(block_number + 10)],
    };
    let receipt = InvokeTransactionReceipt { transaction_hash: tx_hash, ..Default::default() };

    FullBlockWithoutCommitments {
        header: PreconfirmedHeader { block_number, ..Default::default() },
        state_diff: StateDiff {
            nonces: vec![NonceUpdate { contract_address: Felt::from(0x123_u64), nonce: Felt::from(block_number + 1) }],
            ..Default::default()
        },
        transactions: vec![TransactionWithReceipt {
            transaction: InvokeTransactionV0::default().into(),
            receipt: TransactionReceipt::Invoke(receipt),
        }],
        events: vec![EventWithTransactionHash { transaction_hash: tx_hash, event }],
    }
}

#[test]
fn write_block_data_without_header_persists_data_and_staged_marker_without_advancing_tip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = staged_block(0);

    backend.write_access().write_block_data_without_header(&block, &[], None).unwrap();

    assert_eq!(backend.latest_confirmed_block_n(), None);
    assert_eq!(backend.get_max_persisted_block_data().unwrap(), Some(0));
    assert!(backend.db.get_staged_block_header(0).unwrap().is_some());
    assert!(backend.db.get_block_info(0).unwrap().is_none());
    assert_eq!(backend.db.get_block_state_diff(0).unwrap(), Some(block.state_diff.clone()));

    let tx = backend.db.get_transaction(0, 0).unwrap().expect("staged transaction should be persisted");
    assert_eq!(tx.receipt.transaction_hash(), block.transactions[0].receipt.transaction_hash());
    assert_eq!(tx.receipt.events().len(), 1, "events should be merged into persisted transaction receipts");
}

#[test]
fn write_block_data_without_header_rejects_duplicate_staged_block() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = staged_block(0);

    backend.write_access().write_block_data_without_header(&block, &[], None).unwrap();
    assert!(backend.write_access().write_block_data_without_header(&block, &[], None).is_err());
    assert_eq!(backend.get_max_persisted_block_data().unwrap(), Some(0));
    assert!(backend.db.get_staged_block_header(0).unwrap().is_some());
}

#[test]
fn confirm_staged_block_with_precomputed_root_confirms_in_order_and_cleans_marker() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block = staged_block(0);

    backend.write_access().write_block_data_without_header(&block, &[], None).unwrap();
    let state_root = Felt::from(0xdead_beef_u64);
    let result = backend.write_access().confirm_staged_block_with_precomputed_root(0, state_root, false).unwrap();

    assert_eq!(result.new_state_root, state_root);
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert!(backend.db.get_staged_block_header(0).unwrap().is_none());

    let confirmed = backend.block_view_on_confirmed(0).expect("block should be confirmed");
    assert_eq!(confirmed.get_block_info().unwrap().block_hash, result.block_hash);
    assert_eq!(backend.view_on_latest().find_block_by_hash(&result.block_hash).unwrap(), Some(0));
}

#[test]
fn confirm_staged_block_with_precomputed_root_rejects_out_of_order_confirmation() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let block0 = staged_block(0);
    let block1 = staged_block(1);

    backend.write_access().write_block_data_without_header(&block0, &[], None).unwrap();
    backend.write_access().write_block_data_without_header(&block1, &[], None).unwrap();

    let err =
        backend.write_access().confirm_staged_block_with_precomputed_root(1, Felt::from(123_u64), false).unwrap_err();
    assert!(err.to_string().contains("in-order"), "unexpected out-of-order error: {err:#}");

    assert_eq!(backend.latest_confirmed_block_n(), None);
    assert!(backend.db.get_staged_block_header(1).unwrap().is_some());
}
