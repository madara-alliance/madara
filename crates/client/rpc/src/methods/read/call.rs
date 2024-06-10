use jsonrpsee::core::RpcResult;
use mp_convert::field_element::FromFieldElement;
use mp_felt::{FeltWrapper};
use starknet_api::core::{ContractAddress, EntryPointSelector};//, EntryPointSelector};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::Calldata;
use starknet_core::types::{BlockId, FunctionCall};

use crate::utils::execution::block_context;
use crate::utils::{self, ResultExt};
use crate::{Arc, Starknet};

/// Call a Function in a Contract Without Creating a Transaction
///
/// ### Arguments
///
/// * `request` - The details of the function call to be made. This includes information such as the
///   contract address, function signature, and arguments.
/// * `block_id` - The identifier of the block used to reference the state or call the transaction
///   on. This can be the hash of the block, its number (height), or a specific block tag.
///
/// ### Returns
///
/// * `result` - The function's return value, as defined in the Cairo output. This is an array of
///   field elements (`Felt`).
///
/// ### Errors
///
/// This method may return the following errors:
/// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
/// * `CONTRACT_ERROR` - If there is an error with the contract or the function call.
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
pub fn call(starknet: &Starknet, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<String>> {
    let block_info = starknet.get_block_info(block_id)?;
    let block_context = block_context(starknet, &block_info)?;

    let calldata_as_starkfelt = request.calldata.iter().map(
        |x| StarkFelt::from_field_element(*x)
    ).collect();
    let calldata = Calldata(Arc::new(calldata_as_starkfelt));

    let result_as_felt_vector = utils::execution::call_contract(
        ContractAddress::from_field_element(request.contract_address),
        EntryPointSelector(request.entry_point_selector.into_stark_felt()),
        calldata,
        &block_context,
    )
    .or_internal_server_error("Request parameters error")?;

    Ok(
        result_as_felt_vector.iter().map(|x| x.to_string()).collect())
}
