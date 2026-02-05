#![cfg(test)]

use anyhow::Result;
use mp_block::header::{BlockTimestamp, GasPrices, PreconfirmedHeader};
use mp_block::FullBlockWithoutCommitments;
use mp_block::TransactionWithReceipt;
use mp_chain_config::ChainConfig;
use mp_convert::{Felt, ToFelt};
use mp_receipt::{Hash256, L1HandlerTransactionReceipt};
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
use std::sync::Arc;

use crate::{storage::MadaraStorageRead, ChainTip, MadaraBackend};

fn make_l1_handler_tx(nonce: u64) -> L1HandlerTransaction {
    L1HandlerTransaction {
        version: Felt::ZERO,
        nonce,
        contract_address: Felt::from(123u128),
        entry_point_selector: Felt::from(456u128),
        calldata: vec![Felt::from(789u128)].into(),
    }
}

fn make_l1_handler_tx_with_receipt(
    chain_id: Felt,
    protocol_version: mp_chain_config::StarknetVersion,
    nonce: u64,
) -> TransactionWithReceipt {
    let tx = make_l1_handler_tx(nonce);

    let tx_hash = Transaction::L1Handler(tx.clone()).compute_hash(chain_id, protocol_version, false);

    let receipt = L1HandlerTransactionReceipt {
        message_hash: Hash256::from_bytes([0u8; 32]),
        transaction_hash: tx_hash,
        actual_fee: Default::default(),
        messages_sent: vec![],
        events: vec![],
        execution_resources: Default::default(),
        execution_result: Default::default(),
    };

    TransactionWithReceipt { transaction: Transaction::L1Handler(tx), receipt: receipt.into() }
}

fn make_block(
    chain_config: &ChainConfig,
    block_number: u64,
    transactions: Vec<TransactionWithReceipt>,
) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header: PreconfirmedHeader {
            block_number,
            sequencer_address: Felt::ZERO,
            block_timestamp: BlockTimestamp(0),
            protocol_version: chain_config.latest_protocol_version,
            gas_prices: GasPrices::default(),
            l1_da_mode: chain_config.l1_da_mode,
        },
        state_diff: Default::default(),
        transactions,
        events: vec![],
    }
}

#[test]
fn test_revert_restores_l1_to_l2_messages() -> Result<()> {
    let chain_config = Arc::new(ChainConfig::madara_test());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());

    let chain_id = chain_config.chain_id.to_felt();
    let protocol_version = chain_config.latest_protocol_version;

    let block0 = make_block(&chain_config, 0, vec![make_l1_handler_tx_with_receipt(chain_id, protocol_version, 1)]);
    let block0_hash = backend.write_access().add_full_block_with_classes(&block0, &[], false)?.block_hash;

    let block1 = make_block(&chain_config, 1, vec![make_l1_handler_tx_with_receipt(chain_id, protocol_version, 2)]);
    backend.write_access().add_full_block_with_classes(&block1, &[], false)?;

    assert!(backend.get_l1_handler_txn_hash_by_nonce(1)?.is_some());
    assert!(backend.get_l1_handler_txn_hash_by_nonce(2)?.is_some());
    assert!(backend.get_pending_message_to_l2(2)?.is_none());

    backend.revert_to(&block0_hash)?;
    let fresh_chain_tip = backend.db.get_chain_tip()?;
    backend.chain_tip.send_replace(ChainTip::from_storage(fresh_chain_tip));

    assert!(backend.get_l1_handler_txn_hash_by_nonce(2)?.is_none());
    let pending = backend.get_pending_message_to_l2(2)?.expect("pending L1->L2 message should be restored");
    assert_eq!(pending.tx.nonce, 2);

    assert!(backend.get_l1_handler_txn_hash_by_nonce(1)?.is_some());
    assert!(backend.get_pending_message_to_l2(1)?.is_none());

    Ok(())
}

#[test]
fn test_revert_restores_l1_to_l2_messages_with_correct_fee() -> Result<()> {
    let chain_config = Arc::new(ChainConfig::madara_test());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());

    let chain_id = chain_config.chain_id.to_felt();
    let protocol_version = chain_config.latest_protocol_version;

    // First, write pending messages with specific fees (simulating L1 event processing)
    let fee_for_nonce_1 = 1_000_000_000_u128; // 1 gwei
    let fee_for_nonce_2 = 2_500_000_000_u128; // 2.5 gwei

    let msg1 = L1HandlerTransactionWithFee::new(make_l1_handler_tx(1), fee_for_nonce_1);
    let msg2 = L1HandlerTransactionWithFee::new(make_l1_handler_tx(2), fee_for_nonce_2);

    backend.write_pending_message_to_l2(&msg1)?;
    backend.write_pending_message_to_l2(&msg2)?;

    // Add blocks which will confirm the messages and store their fees
    let block0 = make_block(&chain_config, 0, vec![make_l1_handler_tx_with_receipt(chain_id, protocol_version, 1)]);
    let block0_hash = backend.write_access().add_full_block_with_classes(&block0, &[], false)?.block_hash;

    let block1 = make_block(&chain_config, 1, vec![make_l1_handler_tx_with_receipt(chain_id, protocol_version, 2)]);
    backend.write_access().add_full_block_with_classes(&block1, &[], false)?;

    // Verify messages are confirmed and no longer pending
    assert!(backend.get_l1_handler_txn_hash_by_nonce(1)?.is_some());
    assert!(backend.get_l1_handler_txn_hash_by_nonce(2)?.is_some());
    assert!(backend.get_pending_message_to_l2(1)?.is_none());
    assert!(backend.get_pending_message_to_l2(2)?.is_none());

    // Revert to block0
    backend.revert_to(&block0_hash)?;
    let fresh_chain_tip = backend.db.get_chain_tip()?;
    backend.chain_tip.send_replace(ChainTip::from_storage(fresh_chain_tip));

    // Verify message 2 is restored as pending with the CORRECT fee
    let pending = backend.get_pending_message_to_l2(2)?.expect("pending L1->L2 message should be restored");
    assert_eq!(pending.tx.nonce, 2);
    assert_eq!(pending.paid_fee_on_l1, fee_for_nonce_2, "Fee should be restored to original value");

    // Message 1 should still be confirmed
    assert!(backend.get_l1_handler_txn_hash_by_nonce(1)?.is_some());
    assert!(backend.get_pending_message_to_l2(1)?.is_none());

    Ok(())
}
