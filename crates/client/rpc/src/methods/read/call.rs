use jsonrpsee::core::RpcResult;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_api::transaction::Calldata;
use starknet_core::types::{BlockId, FunctionCall};

use crate::utils::execution::block_context;
use crate::utils::helpers::previous_substrate_block_hash;
use crate::errors::StarknetRpcApiError;
use crate::{utils, Arc, Starknet};

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
pub fn call<BE, C, H>(starknet: &Starknet<BE, C, H>, request: FunctionCall, block_id: BlockId) -> RpcResult<Vec<String>>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let previous_substrate_block_hash = previous_substrate_block_hash(starknet, substrate_block_hash)?;
    let block_context = block_context(starknet.client.as_ref(), previous_substrate_block_hash)?;

    let calldata = Calldata(Arc::new(request.calldata.iter().map(|x| Felt252Wrapper::from(*x).into()).collect()));

    let result = utils::execution::call_contract(
        Felt252Wrapper(request.contract_address).into(),
        Felt252Wrapper(request.entry_point_selector).into(),
        calldata,
        &block_context,
    )
    .map_err(|_| {
        log::error!("Request parameters error");
        StarknetRpcApiError::InternalServerError
    })?;

    // let result = convert_error(starknet.client.clone(), substrate_block_hash, result)?;

    Ok(result.iter().map(|x| format!("{:#x}", x.0)).collect())
}
