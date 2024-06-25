use dp_convert::ToStarkFelt;
use dp_transactions::L1HandlerTransaction;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::{Fee, TransactionHash};
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};
use starknet_types_core::felt::Felt;

use crate::utils::execution::block_context;
use crate::utils::ResultExt;
use crate::{utils, Starknet};

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
) -> RpcResult<FeeEstimate> {
    let block_info = starknet.get_block_info(block_id)?;
    let block_context = block_context(starknet, &block_info)?;
    let block_number = block_info.block_n();

    let chain_id = starknet.chain_config.chain_id;
    let transaction = convert_message_into_l1_handler(message, chain_id, Some(block_number));

    let message_fee = utils::execution::estimate_message_fee(starknet, transaction, &block_context)
        .or_contract_error("Function execution failed")?;

    Ok(message_fee)
}

pub fn convert_message_into_l1_handler(
    message: MsgFromL1,
    chain_id: Felt,
    block_number: Option<u64>,
) -> blockifier::transaction::transactions::L1HandlerTransaction {
    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(chain_id, false, block_number);
    let tx: starknet_api::transaction::L1HandlerTransaction = (&l1_handler).try_into().unwrap();

    blockifier::transaction::transactions::L1HandlerTransaction {
        tx,
        tx_hash: TransactionHash(tx_hash.to_stark_felt()),
        paid_fee_on_l1: Fee(1),
    }
}
