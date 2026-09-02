#![cfg(test)]

use crate::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use crate::storage::MadaraStorageRead;
use crate::{MadaraBackend, MadaraBackendConfig};
use mp_block::header::PreconfirmedHeader;
use mp_block::{Transaction, TransactionWithReceipt};
use mp_chain_config::ChainConfig;
use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
use mp_state_update::TransactionStateUpdate;
use mp_transactions::InvokeTransactionV0;
use rstest::rstest;
use std::sync::Arc;

fn dummy_executed_tx() -> PreconfirmedExecutedTransaction {
    PreconfirmedExecutedTransaction {
        transaction: TransactionWithReceipt {
            transaction: Transaction::Invoke(mp_transactions::InvokeTransaction::V0(InvokeTransactionV0::default())),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt::default()),
        },
        state_diff: TransactionStateUpdate {
            storage_diffs: Default::default(),
            contract_class_hashes: Default::default(),
            declared_classes: Default::default(),
            nonces: Default::default(),
        },
        declared_class: None,
        arrived_at: Default::default(),
        paid_fee_on_l1: None,
    }
}

#[rstest]
fn confirmed_path_immediate_gc_for_preconfirmed_rows() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    let block_n = 0u64;

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: block_n, ..Default::default() }))
        .expect("new preconfirmed");
    backend
        .write_access()
        .append_to_preconfirmed(block_n, &[dummy_executed_tx()], [])
        .expect("append preconfirmed content");

    let before = backend.db.get_preconfirmed_block_data(block_n).expect("read preconfirmed before close");
    assert!(before.is_some(), "preconfirmed rows must exist before confirmation");

    backend.write_access().new_confirmed_block(block_n).expect("confirm block");

    let after = backend.db.get_preconfirmed_block_data(block_n).expect("read preconfirmed after close");
    assert!(after.is_none(), "preconfirmed rows <= confirmed tip must be GC'd immediately");
}

#[test]
fn appends_are_routed_by_explicit_preconfirmed_block_number() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: false, ..Default::default() },
    );

    for block_n in 0..=1 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: block_n,
                ..Default::default()
            }))
            .expect("new preconfirmed block");
    }

    backend
        .write_access()
        .append_to_preconfirmed(0, &[dummy_executed_tx()], [])
        .expect("append must target block 0 instead of the latest runtime tip");
    assert_eq!(backend.block_view_on_preconfirmed(0).expect("block 0 runtime view").block().transaction_count(), 1);
    assert_eq!(backend.block_view_on_preconfirmed(1).expect("block 1 runtime view").block().transaction_count(), 0);

    backend.write_access().new_confirmed_block(0).expect("confirm block 0");
    let error = backend
        .write_access()
        .append_to_preconfirmed(0, &[dummy_executed_tx()], [])
        .expect_err("a delayed batch must not fall through to block 1");
    assert!(error.to_string().contains("There is no preconfirmed block #0"));
}
