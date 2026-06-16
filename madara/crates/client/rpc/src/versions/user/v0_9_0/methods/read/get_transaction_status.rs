use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_receipt::ExecutionResult;
use mp_rpc::v0_9_0::{TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnStatus};
use starknet_types_core::felt::Felt;

/// Gets the status of a transaction.
///
/// Supported statuses are:
///
/// - [`Received`]: tx has been inserted into the mempool.
/// - [`Candidate`]: tx is a candidate for the pre-confirmed block.
/// - [`PreConfirmed`]: tx is executed in the pre-confirmed block.
/// - [`AcceptedOnL2`]: tx has been saved to the pending block.
/// - [`AcceptedOnL1`]: tx has been finalized on L1.
///
/// [`Received`]: mp_rpc::v0_9_0::TxnStatus::Received
/// [`Candidate`]: mp_rpc::v0_9_0::TxnStatus::Candidate
/// [`PreConfirmed`]: mp_rpc::v0_9_0::TxnStatus::PreConfirmed
/// [`AcceptedOnL2`]: mp_rpc::v0_9_0::TxnStatus::AcceptedOnL2
/// [`AcceptedOnL1`]: mp_rpc::v0_9_0::TxnStatus::AcceptedOnL1
pub async fn get_transaction_status(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TxnFinalityAndExecutionStatus> {
    let view = starknet.backend.view_on_latest();
    // TODO: rpc v0.9 Candidate status.
    if let Some(res) = view.find_transaction_by_hash(&transaction_hash)? {
        let transaction = res.get_transaction()?;

        let (execution_status, failure_reason) = match transaction.receipt.execution_result() {
            ExecutionResult::Reverted { reason } => (Some(TxnExecutionStatus::Reverted), Some(reason.clone())),
            ExecutionResult::Succeeded => (Some(TxnExecutionStatus::Succeeded), None),
        };

        let finality_status = if res.block.is_on_l1() {
            TxnStatus::AcceptedOnL1
        } else if res.block.is_confirmed() {
            TxnStatus::AcceptedOnL2
        } else {
            TxnStatus::PreConfirmed
        };

        Ok(TxnFinalityAndExecutionStatus { finality_status, execution_status, failure_reason })
    } else if starknet.transaction_lookup.received_transaction(transaction_hash).await.is_some_and(|b| b) {
        Ok(TxnFinalityAndExecutionStatus {
            finality_status: TxnStatus::Received,
            execution_status: None,
            failure_reason: None,
        })
    } else {
        Err(StarknetRpcApiError::TxnHashNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TX_HASH: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked(
        "0x3ccaabf599097d1965e1ef8317b830e76eb681016722c9364ed6e59f3252908",
    );

    #[rstest::fixture]
    fn logs() {
        let debug = tracing_subscriber::filter::LevelFilter::DEBUG;
        let env = tracing_subscriber::EnvFilter::builder().with_default_directive(debug.into()).from_env_lossy();
        let timer = tracing_subscriber::fmt::time::Uptime::default();
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(env)
            .with_file(true)
            .with_line_number(true)
            .with_target(false)
            .with_timer(timer)
            .try_init();
    }

    #[rstest::fixture]
    fn tx() -> mp_rpc::v0_9_0::BroadcastedInvokeTxn {
        mp_rpc::v0_9_0::BroadcastedInvokeTxn::V3(mp_rpc::v0_9_0::InvokeTxnV3 {
            calldata: Default::default(),
            sender_address: Default::default(),
            signature: Default::default(),
            nonce: Default::default(),
            resource_bounds: mp_rpc::v0_9_0::ResourceBoundsMapping {
                l1_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l1_data_gas: mp_rpc::v0_9_0::ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            },
            tip: Default::default(),
            paymaster_data: Default::default(),
            account_deployment_data: Default::default(),
            nonce_data_availability_mode: mp_rpc::v0_9_0::DaMode::L1,
            fee_data_availability_mode: mp_rpc::v0_9_0::DaMode::L1,
        })
    }

    #[rstest::fixture]
    fn tx_with_receipt(tx: mp_rpc::v0_9_0::BroadcastedInvokeTxn) -> mp_block::TransactionWithReceipt {
        mp_block::TransactionWithReceipt {
            transaction: mp_transactions::Transaction::Invoke(tx.into()),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: TX_HASH,
                execution_result: mp_receipt::ExecutionResult::Succeeded,
                ..Default::default()
            }),
        }
    }

    #[rstest::fixture]
    fn block(tx_with_receipt: mp_block::TransactionWithReceipt) -> mp_block::FullBlockWithoutCommitments {
        mp_block::FullBlockWithoutCommitments {
            header: Default::default(),
            state_diff: Default::default(),
            transactions: vec![tx_with_receipt],
            events: Default::default(),
        }
    }

    #[rstest::fixture]
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

        Starknet::new(
            backend,
            std::sync::Arc::clone(&mempool_validator) as _,
            mempool_validator,
            Default::default(),
            None,
            context,
        )
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_received(_logs: (), starknet: Starknet, tx: mp_rpc::v0_9_0::BroadcastedInvokeTxn) {
        let provider = std::sync::Arc::clone(&starknet.transaction_submitter);
        let result = provider.submit_invoke_transaction(tx.into()).await.expect("Failed to submit invoke transaction");
        let tx_hash = result.transaction_hash;

        let status = get_transaction_status(&starknet, tx_hash).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::Received,
                execution_status: None,
                failure_reason: None
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_accepted_on_l2(
        _logs: (),
        starknet: Starknet,
        block: mp_block::FullBlockWithoutCommitments,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).expect("Failed to store pending block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(mp_rpc::v0_9_0::TxnExecutionStatus::Succeeded),
                failure_reason: None
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_pre_confirmed(
        _logs: (),
        starknet: Starknet,
        tx_with_receipt: mp_block::TransactionWithReceipt,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);
        let preconfirmed = mc_db::preconfirmed::PreconfirmedBlock::new_with_content(
            mp_block::header::PreconfirmedHeader { block_number: 0, ..Default::default() },
            [mc_db::preconfirmed::PreconfirmedExecutedTransaction {
                transaction: tx_with_receipt,
                state_diff: Default::default(),
                declared_class: None,
                arrived_at: Default::default(),
                paid_fee_on_l1: None,
            }],
            std::iter::empty(),
        );
        backend.write_access().new_preconfirmed(preconfirmed).expect("Failed to store pre-confirmed block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::PreConfirmed,
                execution_status: Some(mp_rpc::v0_9_0::TxnExecutionStatus::Succeeded),
                failure_reason: None
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_accepted_on_l1(
        _logs: (),
        starknet: Starknet,
        block: mp_block::FullBlockWithoutCommitments,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).expect("Failed to store pending block");
        backend.set_latest_l1_confirmed(Some(0)).expect("Failed to update last confirmed block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL1,
                execution_status: Some(mp_rpc::v0_9_0::TxnExecutionStatus::Succeeded),
                failure_reason: None
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_err_not_found(_logs: (), starknet: Starknet) {
        assert_eq!(get_transaction_status(&starknet, TX_HASH).await, Err(StarknetRpcApiError::TxnHashNotFound));
    }

    /// Reverted transactions must carry the failure reason in the status result.
    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_reverted_includes_failure_reason(
        _logs: (),
        starknet: Starknet,
        tx: mp_rpc::v0_9_0::BroadcastedInvokeTxn,
    ) {
        let block = mp_block::FullBlockWithoutCommitments {
            header: Default::default(),
            state_diff: Default::default(),
            transactions: vec![mp_block::TransactionWithReceipt {
                transaction: mp_transactions::Transaction::Invoke(tx.into()),
                receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                    transaction_hash: TX_HASH,
                    execution_result: mp_receipt::ExecutionResult::Reverted { reason: "Insufficient fee".into() },
                    ..Default::default()
                }),
            }],
            events: Default::default(),
        };
        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).expect("Failed to store block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(mp_rpc::v0_9_0::TxnExecutionStatus::Reverted),
                failure_reason: Some("Insufficient fee".into())
            }
        );
    }
}
