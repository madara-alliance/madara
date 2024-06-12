use dp_felt::Felt252Wrapper;
use jsonrpsee::core::RpcResult;
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

    let calldata = Calldata(Arc::new(request.calldata.iter().map(|x| Felt252Wrapper::from(*x).into()).collect()));

    let result = utils::execution::call_contract(
        Felt252Wrapper(request.contract_address).into(),
        Felt252Wrapper(request.entry_point_selector).into(),
        calldata,
        &block_context,
    )
    .or_internal_server_error("Request parameters error")?;

    Ok(result.iter().map(|x| format!("{:#x}", x.0)).collect())
}
