#![cfg(any(test, feature = "testing"))]

use crate::MadaraBackend;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
use mp_convert::Felt;
use mp_receipt::{L1HandlerTransactionReceipt, TransactionReceipt};
use mp_transactions::{L1HandlerTransaction, Transaction};
use std::sync::Arc;

pub fn add_test_block(
    backend: &Arc<MadaraBackend>,
    block_number: u64,
    transactions: Vec<TransactionWithReceipt>,
) -> Felt {
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number, ..Default::default() },
                state_diff: Default::default(),
                transactions,
                events: vec![],
            },
            &[],
            false,
        )
        .expect("Adding block should succeed")
        .block_hash
}

pub fn l1_handler_tx_with_receipt(nonce: u64, tx_hash: Felt) -> TransactionWithReceipt {
    TransactionWithReceipt {
        transaction: Transaction::L1Handler(L1HandlerTransaction { nonce, ..Default::default() }),
        receipt: TransactionReceipt::L1Handler(L1HandlerTransactionReceipt {
            transaction_hash: tx_hash,
            ..Default::default()
        }),
    }
}
