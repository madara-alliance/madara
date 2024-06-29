use dc_exec::{block_context, execute_transactions, execution_result_to_fee_estimate};
use dp_convert::ToStarkFelt;
use dp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::methods::trace::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
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
    let block_info = starknet.get_block_info(block_id)?;
    if block_info.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let block_context = block_context(block_info.header(), &starknet.chain_id());
    let block_number = block_info.block_n();

    let chain_id = starknet.chain_id();
    let transaction = convert_message_into_transaction(message, chain_id, Some(block_number));
    let execution_result =
        execute_transactions(starknet.clone_backend(), vec![], vec![transaction], &block_context, false, true)?
            .pop()
            .ok_or_else(|| StarknetRpcApiError::ErrUnexpectedError { data: "No execution result found".to_string() })?;

    let fee_estimate = execution_result_to_fee_estimate(&execution_result, &block_context);

    Ok(fee_estimate)
}

pub fn convert_message_into_transaction(
    message: MsgFromL1,
    chain_id: Felt,
    block_number: Option<u64>,
) -> blockifier::transaction::transaction_execution::Transaction {
    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(chain_id, false, block_number);
    let tx: starknet_api::transaction::L1HandlerTransaction = (&l1_handler).try_into().unwrap();

    let tx = blockifier::transaction::transactions::L1HandlerTransaction {
        tx,
        tx_hash: TransactionHash(tx_hash.to_stark_felt()),
        paid_fee_on_l1: Fee(1),
    };
    blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(tx)
}
