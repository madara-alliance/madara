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
    backend.write_access().append_to_preconfirmed(&[dummy_executed_tx()], []).expect("append preconfirmed content");

    let before = backend.db.get_preconfirmed_block_data(block_n).expect("read preconfirmed before close");
    assert!(before.is_some(), "preconfirmed rows must exist before confirmation");

    backend.write_access().new_confirmed_block(block_n).expect("confirm block");

    let after = backend.db.get_preconfirmed_block_data(block_n).expect("read preconfirmed after close");
    assert!(after.is_none(), "preconfirmed rows <= confirmed tip must be GC'd immediately");
}
