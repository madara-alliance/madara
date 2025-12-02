use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use anyhow::Context;
use mc_exec::execution::TxInfo;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_10_0::{BlockId, MessageFeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{fields::Fee, TransactionHash};

/// Estimates the L2 fee for an L1 message.
///
/// v0.10.0: Returns `CONTRACT_NOT_FOUND` if the L1 handler contract (`to_address`) doesn't exist on L2.
pub async fn estimate_message_fee(
    starknet: &Starknet,
    message: MsgFromL1,
    block_id: BlockId,
) -> StarknetRpcResult<MessageFeeEstimate> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    let view = starknet.resolve_block_view(block_id)?;
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = view.state_view();
    if state_view.get_contract_class_hash(&message.to_address)?.is_none() {
        return Err(StarknetRpcApiError::ContractNotFound {
            error: format!("Contract at address {:#x} not found", message.to_address).into(),
        });
    }

    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(
        view.backend().chain_config().chain_id.to_felt(),
        /* offset_version */ false,
        /* legacy */ false,
    );
    let tx: starknet_api::transaction::L1HandlerTransaction = l1_handler.try_into().unwrap();
    let transaction = blockifier::transaction::transaction_execution::Transaction::L1Handler(
        starknet_api::executable_transaction::L1HandlerTransaction {
            tx,
            tx_hash: TransactionHash(tx_hash),
            paid_fee_on_l1: Fee::default(),
        },
    );

    let tip = transaction.tip().unwrap_or_default();
    let (mut execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], [transaction])?, exec_context))
    })
    .await?;

    let execution_result = execution_results.pop().context("There should be one result")?;

    let fee_estimate = exec_context.execution_result_to_fee_estimate_v0_9(&execution_result, tip)?;

    Ok(MessageFeeEstimate { common: fee_estimate, unit: mp_rpc::v0_10_0::PriceUnitWei::Wei })
}
