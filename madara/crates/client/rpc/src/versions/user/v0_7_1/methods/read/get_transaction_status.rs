use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use mp_receipt::ExecutionResult;
use mp_rpc::{TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnStatus};
use starknet_types_core::felt::Felt;

/// Gets the status of a transaction. ([specs])
///
/// Supported statuses are:
///
/// - [`Received`]: tx has been inserted into the mempool.
/// - [`AcceptedOnL2`]: tx has been saved to the pending block.
/// - [`AcceptedOnL1`]: tx has been finalized on L1.
///
/// [specs]: https://github.com/starkware-libs/starknet-specs/blob/a2d10fc6cbaddbe2d3cf6ace5174dd0a306f4885/api/starknet_api_openrpc.json#L224C5-L250C7
/// [`Received`]: mp_rpc::v0_7_1::TxnStatus::Received
/// [`AcceptedOnL2`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
/// [`AcceptedOnL1`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
pub async fn get_transaction_status(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TxnFinalityAndExecutionStatus> {
    let view = starknet.backend.view_on_latest();
    // TODO: rpc v0.9 Candidate status.
    if let Some(res) = view.find_transaction_by_hash(&transaction_hash)? {
        let transaction = res.get_transaction()?;

        let execution_status = match transaction.receipt.execution_result() {
            ExecutionResult::Reverted { .. } => Some(TxnExecutionStatus::Reverted),
            ExecutionResult::Succeeded => Some(TxnExecutionStatus::Succeeded),
        };

        let finality_status = if res.block.is_on_l1() {
            TxnStatus::AcceptedOnL1
        } else if res.block.is_confirmed() {
            TxnStatus::AcceptedOnL2
        } else {
            // FIXME: rpc v0.9 TxnStatus::Preconfirmed
            TxnStatus::AcceptedOnL2
        };

        Ok(TxnFinalityAndExecutionStatus { finality_status, execution_status })
    } else if starknet.add_transaction_provider.received_transaction(transaction_hash).await.is_some_and(|b| b) {
        Ok(TxnFinalityAndExecutionStatus { finality_status: TxnStatus::Received, execution_status: None })
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
    fn tx() -> mp_rpc::BroadcastedInvokeTxn {
        mp_rpc::BroadcastedInvokeTxn::V0(mp_rpc::InvokeTxnV0 {
            calldata: Default::default(),
            contract_address: Default::default(),
            entry_point_selector: Default::default(),
            max_fee: Default::default(),
            signature: Default::default(),
        })
    }

    #[rstest::fixture]
    fn tx_with_receipt(tx: mp_rpc::BroadcastedInvokeTxn) -> mp_block::TransactionWithReceipt {
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
    fn block(tx_with_receipt: mp_block::TransactionWithReceipt) -> mp_block::PreconfirmedFullBlock {
        mp_block::PreconfirmedFullBlock {
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

        Starknet::new(backend, mempool_validator, Default::default(), None, context)
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_received(_logs: (), starknet: Starknet, tx: mp_rpc::BroadcastedInvokeTxn) {
        let provider = std::sync::Arc::clone(&starknet.add_transaction_provider);
        provider.submit_invoke_transaction(tx).await.expect("Failed to submit invoke transaction");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus { finality_status: TxnStatus::Received, execution_status: None }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_accepted_on_l2(
        _logs: (),
        starknet: Starknet,
        block: mp_block::PreconfirmedFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).expect("Failed to store pending block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(mp_rpc::v0_7_1::TxnExecutionStatus::Succeeded)
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_accepted_on_l1(
        _logs: (),
        starknet: Starknet,
        block: mp_block::PreconfirmedFullBlock,
    ) {
        let backend = std::sync::Arc::clone(&starknet.backend);
        backend.write_access().add_full_block_with_classes(&block, &[], true).expect("Failed to store pending block");
        backend.set_latest_l1_confirmed(Some(0)).expect("Failed to update last confirmed block");

        let status = get_transaction_status(&starknet, TX_HASH).await.expect("Failed to retrieve transaction status");

        assert_eq!(
            status,
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL1,
                execution_status: Some(mp_rpc::v0_7_1::TxnExecutionStatus::Succeeded)
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_transaction_status_err_not_found(_logs: (), starknet: Starknet) {
        assert_eq!(get_transaction_status(&starknet, TX_HASH).await, Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
