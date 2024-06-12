use std::sync::Arc;

use blockifier::transaction::transactions::L1HandlerTransaction;
use dp_felt::{Felt252Wrapper, FeltWrapper};
use dp_transactions::compute_hash::ComputeTransactionHash;
use jsonrpsee::core::RpcResult;
use starknet_api::core::Nonce;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{Calldata, Fee, TransactionVersion};
use starknet_core::types::{BlockId, FeeEstimate, MsgFromL1};

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

    let transaction = convert_message_into_tx(message, starknet.chain_config.chain_id.0.into(), Some(block_number));

    let message_fee = utils::execution::estimate_message_fee(transaction, &block_context)
        .or_contract_error("Function execution failed")?;

    Ok(message_fee)
}

pub fn convert_message_into_tx(
    message: MsgFromL1,
    chain_id: Felt252Wrapper,
    block_number: Option<u64>,
) -> L1HandlerTransaction {
    let calldata = std::iter::once(message.from_address.into_stark_felt())
        .chain(message.payload.into_iter().map(FeltWrapper::into_stark_felt))
        .collect();
    let tx = starknet_api::transaction::L1HandlerTransaction {
        version: TransactionVersion::ZERO,
        nonce: Nonce(StarkFelt::ZERO),
        contract_address: Felt252Wrapper::from(message.to_address).into(),
        entry_point_selector: Felt252Wrapper::from(message.entry_point_selector).into(),
        calldata: Calldata(Arc::new(calldata)),
    };
    // TODO(merge): recheck if this is correct
    let tx_hash = tx.compute_hash(chain_id, true, block_number);

    L1HandlerTransaction { tx, tx_hash, paid_fee_on_l1: Fee(10) } //TODO: fix with real fee
}
