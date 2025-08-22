use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use anyhow::Context;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_block::BlockId;
use mp_convert::ToFelt;
use mp_rpc::{FeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{fields::Fee, TransactionHash};
use starknet_types_core::felt::Felt;

/// Estimate the L2 fee of a message sent on L1
///
/// # Arguments
///
/// * `message` - the message to estimate
/// * `block_id` - hash, number (height), or tag of the requested block
///
/// # Returns
///
/// * `FeeEstimate` - the fee estimation (gas consumed, gas price, overall fee, unit)
///
/// # Errors
///
/// BlockNotFound : If the specified block does not exist.
/// ContractNotFound : If the specified contract address does not exist.
/// ContractError : If there is an error with the contract.
pub async fn estimate_message_fee(
    starknet: &Starknet,
    message: MsgFromL1,
    block_id: BlockId,
) -> StarknetRpcResult<FeeEstimate> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    let view = starknet.backend.block_view(block_id)?;
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let transaction = convert_message_into_transaction(message, view.backend().chain_config().chain_id.to_felt());

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (mut execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], [transaction])?, exec_context))
    })
    .await?;

    let execution_result =
        execution_results.pop().context("There should be at least one result")?;

    let fee_estimate = exec_context.execution_result_to_fee_estimate(&execution_result);

    Ok(fee_estimate)
}

pub fn convert_message_into_transaction(
    message: MsgFromL1,
    chain_id: Felt,
) -> blockifier::transaction::transaction_execution::Transaction {
    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(chain_id, /* offset_version */ false, /* legacy */ false);
    // TODO: remove this unwrap
    let tx: starknet_api::transaction::L1HandlerTransaction = l1_handler.try_into().unwrap();

    blockifier::transaction::transaction_execution::Transaction::L1Handler(
        starknet_api::executable_transaction::L1HandlerTransaction {
            tx,
            tx_hash: TransactionHash(tx_hash),
            paid_fee_on_l1: Fee::default(),
        },
    )
}
