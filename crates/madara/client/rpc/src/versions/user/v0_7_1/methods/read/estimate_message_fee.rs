use std::sync::Arc;

use mc_exec::ExecutionContext;
use mp_block::BlockId;
use mp_rpc::{FeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_types_core::felt::Felt;

use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::OptionExt;
use crate::versions::user::v0_7_1::methods::trace::trace_transaction::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use crate::Starknet;

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
    let block_info = starknet.get_block_info(&block_id)?;

    if block_info.protocol_version() < &EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&starknet.backend), &block_info)?;

    let transaction = convert_message_into_transaction(message, starknet.chain_id());
    let execution_result = exec_context
        .re_execute_transactions([], [transaction], false, true)?
        .pop()
        .ok_or_internal_server_error("Failed to convert BroadcastedTransaction to AccountTransaction")?;

    let fee_estimate = exec_context.execution_result_to_fee_estimate(&execution_result);

    Ok(fee_estimate)
}

pub fn convert_message_into_transaction(
    message: MsgFromL1,
    chain_id: Felt,
) -> blockifier::transaction::transaction_execution::Transaction {
    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(chain_id, false, false);
    let tx: starknet_api::transaction::L1HandlerTransaction = l1_handler.try_into().unwrap();

    let tx = blockifier::transaction::transactions::L1HandlerTransaction {
        tx,
        tx_hash: TransactionHash(tx_hash),
        paid_fee_on_l1: Fee(1),
    };
    blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(tx)
}
