use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_receipt::ExecutionResult;
use mp_rpc::v0_9_0::{L1TxnHash, MessageStatus, TxnExecutionStatus, TxnFinalityStatus};

/// `starknet_getMessagesStatus`.
pub fn get_messages_status(starknet: &Starknet, transaction_hash: L1TxnHash) -> StarknetRpcResult<Vec<MessageStatus>> {
    let l1_tx_hash = mp_convert::L1TransactionHash(transaction_hash.0);

    let Some(messages) = starknet.backend.get_messages_to_l2_by_l1_tx_hash(&l1_tx_hash)? else {
        return Err(StarknetRpcApiError::TxnHashNotFound);
    };

    let view = starknet.backend.view_on_latest();

    let mut out = Vec::new();
    for (_core_contract_nonce, maybe_l2_tx_hash) in messages {
        let Some(l2_tx_hash) = maybe_l2_tx_hash else {
            continue;
        };

        let Some(res) = view.find_transaction_by_hash(&l2_tx_hash)? else {
            continue;
        };

        let transaction = res.get_transaction()?;

        let finality_status = if res.block.is_preconfirmed() {
            TxnFinalityStatus::PreConfirmed
        } else if res.block.is_on_l1() {
            TxnFinalityStatus::L1
        } else {
            TxnFinalityStatus::L2
        };

        let (execution_status, failure_reason) = match transaction.receipt.execution_result() {
            ExecutionResult::Succeeded => (TxnExecutionStatus::Succeeded, None),
            ExecutionResult::Reverted { reason } => (TxnExecutionStatus::Reverted, Some(reason.clone())),
        };

        // Optional field only for REVERTED, per spec.
        let failure_reason = match execution_status {
            TxnExecutionStatus::Reverted => failure_reason,
            TxnExecutionStatus::Succeeded => None,
        };

        out.push(MessageStatus { transaction_hash: l2_tx_hash, finality_status, execution_status, failure_reason });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starknet() -> Starknet {
        let chain_config = std::sync::Arc::new(mp_chain_config::ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let validation = mc_submit_tx::TransactionValidatorConfig { disable_validation: true, disable_fee: false };
        let mempool = std::sync::Arc::new(mc_mempool::Mempool::new(
            std::sync::Arc::clone(&backend),
            mc_mempool::MempoolConfig::default(),
        ));
        let mempool_validator = std::sync::Arc::new(mc_submit_tx::TransactionValidator::new(
            mempool,
            std::sync::Arc::clone(&backend),
            validation,
        ));
        let context = mp_utils::service::ServiceContext::new_for_testing();

        Starknet::new(backend, mempool_validator, Default::default(), None, context)
    }

    fn l1_hash_bytes(x: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[31] = x;
        b
    }

    #[test]
    fn get_messages_status_err_not_found() {
        let starknet = starknet();
        let res = get_messages_status(&starknet, L1TxnHash(l1_hash_bytes(1)));
        assert_eq!(res, Err(StarknetRpcApiError::TxnHashNotFound));
    }

    #[test]
    fn get_messages_status_empty_when_only_seen_marker() {
        let starknet = starknet();
        let l1 = mp_convert::L1TransactionHash(l1_hash_bytes(1));
        assert!(starknet.backend.insert_message_to_l2_seen_marker(&l1, 7).unwrap());

        let res = get_messages_status(&starknet, L1TxnHash(l1.0)).unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn get_messages_status_includes_consumed_tx_with_revert_reason() {
        let starknet = starknet();
        let l1 = mp_convert::L1TransactionHash(l1_hash_bytes(1));
        let l2_tx_hash = starknet_types_core::felt::Felt::from_hex_unchecked("0x123");

        // Insert the message index entry.
        assert!(starknet.backend.insert_message_to_l2_seen_marker(&l1, 7).unwrap());
        starknet.backend.write_message_to_l2_consumed_txn_hash(&l1, 7, &l2_tx_hash).unwrap();

        // Store an L1 handler tx with a reverted receipt.
        let tx = mp_transactions::L1HandlerTransaction { nonce: 7, ..Default::default() };
        let receipt = mp_receipt::TransactionReceipt::L1Handler(mp_receipt::L1HandlerTransactionReceipt {
            transaction_hash: l2_tx_hash,
            execution_result: mp_receipt::ExecutionResult::Reverted { reason: "boom".to_string() },
            ..Default::default()
        });
        let block = mp_block::FullBlockWithoutCommitments {
            header: Default::default(),
            state_diff: Default::default(),
            transactions: vec![mp_block::TransactionWithReceipt { transaction: tx.into(), receipt }],
            events: Default::default(),
        };

        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).unwrap();

        let res = get_messages_status(&starknet, L1TxnHash(l1.0)).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].transaction_hash, l2_tx_hash);
        assert_eq!(res[0].execution_status, TxnExecutionStatus::Reverted);
        assert_eq!(res[0].failure_reason.as_deref(), Some("boom"));
    }

    #[test]
    fn get_messages_status_returns_multiple_consumed_messages() {
        let starknet = starknet();
        let l1 = mp_convert::L1TransactionHash(l1_hash_bytes(1));
        let l2_tx_hash_1 = starknet_types_core::felt::Felt::from_hex_unchecked("0x123");
        let l2_tx_hash_2 = starknet_types_core::felt::Felt::from_hex_unchecked("0x456");

        // Insert the message index entries.
        assert!(starknet.backend.insert_message_to_l2_seen_marker(&l1, 7).unwrap());
        assert!(starknet.backend.insert_message_to_l2_seen_marker(&l1, 8).unwrap());
        starknet.backend.write_message_to_l2_consumed_txn_hash(&l1, 7, &l2_tx_hash_1).unwrap();
        starknet.backend.write_message_to_l2_consumed_txn_hash(&l1, 8, &l2_tx_hash_2).unwrap();

        // Store two L1 handler transactions in the same block.
        let tx_1 = mp_transactions::L1HandlerTransaction { nonce: 7, ..Default::default() };
        let tx_2 = mp_transactions::L1HandlerTransaction { nonce: 8, ..Default::default() };
        let receipt_1 = mp_receipt::TransactionReceipt::L1Handler(mp_receipt::L1HandlerTransactionReceipt {
            transaction_hash: l2_tx_hash_1,
            execution_result: mp_receipt::ExecutionResult::Succeeded,
            ..Default::default()
        });
        let receipt_2 = mp_receipt::TransactionReceipt::L1Handler(mp_receipt::L1HandlerTransactionReceipt {
            transaction_hash: l2_tx_hash_2,
            execution_result: mp_receipt::ExecutionResult::Succeeded,
            ..Default::default()
        });
        let block = mp_block::FullBlockWithoutCommitments {
            header: Default::default(),
            state_diff: Default::default(),
            transactions: vec![
                mp_block::TransactionWithReceipt { transaction: tx_1.into(), receipt: receipt_1 },
                mp_block::TransactionWithReceipt { transaction: tx_2.into(), receipt: receipt_2 },
            ],
            events: Default::default(),
        };

        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).unwrap();

        let res = get_messages_status(&starknet, L1TxnHash(l1.0)).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].transaction_hash, l2_tx_hash_1);
        assert_eq!(res[1].transaction_hash, l2_tx_hash_2);
    }
}
