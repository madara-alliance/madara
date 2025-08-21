use std::sync::Arc;

use mc_exec::ExecutionContext;
use mp_block::BlockId;
use mp_rpc::v0_8_1::{FeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{fields::Fee, TransactionHash};
use starknet_types_core::felt::Felt;

use crate::constants::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::OptionExt;
use crate::Starknet;

pub async fn estimate_message_fee(
    starknet: &Starknet,
    message: MsgFromL1,
    block_id: BlockId,
) -> StarknetRpcResult<FeeEstimate> {
    let block_info = starknet.get_block_info(&block_id)?;

    if block_info.protocol_version() < &EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&starknet.backend), &block_info)?;

    let transaction = convert_message_into_transaction(message, starknet.chain_id());
    let execution_result = exec_context
        .re_execute_transactions([], [transaction])?
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
