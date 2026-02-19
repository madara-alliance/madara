#![cfg(test)]

use crate::storage::MadaraStorageRead;
use crate::MadaraBackend;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_receipt::{Event, EventWithTransactionHash, InvokeTransactionReceipt};
use mp_state_update::StateDiff;
use mp_transactions::InvokeTransactionV0;

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
async fn parallel_merkle_checkpoint_metadata_roundtrip() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());

    backend.write_parallel_merkle_checkpoint(5).unwrap();
    backend.write_parallel_merkle_checkpoint(8).unwrap();

    assert!(backend.has_parallel_merkle_checkpoint(5).unwrap());
    assert!(backend.has_parallel_merkle_checkpoint(8).unwrap());
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().unwrap(), Some(8));
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
