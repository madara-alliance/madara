use jsonrpsee::core::RpcResult;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockHashAndNumber, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Get the Most Recent Accepted Block Hash and Number
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `block_hash_and_number` - A tuple containing the latest block hash and number of the current
///   network.
pub fn block_hash_and_number<BE, C, H>(starknet: &Starknet<BE, C, H>) -> RpcResult<BlockHashAndNumber>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let block_number = starknet.current_block_number()?;
    let block_hash = starknet.current_block_hash().map_err(|e| {
        log::error!("Failed to retrieve the current block hash: {}", e);
        StarknetRpcApiError::NoBlocks
    })?;

    Ok(BlockHashAndNumber {
        block_hash: FieldElement::from_byte_slice_be(block_hash.as_bytes()).unwrap(),
        block_number,
    })
}
