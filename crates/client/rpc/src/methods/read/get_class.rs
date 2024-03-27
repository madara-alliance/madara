use jsonrpsee::core::RpcResult;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_contract::class::ContractClassWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Get the contract class definition in the given block associated with the given hash.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag.
/// * `class_hash` - The hash of the requested contract class.
///
/// ### Returns
///
/// Returns the contract class definition if found. In case of an error, returns a
/// `StarknetRpcApiError` indicating either `BlockNotFound` or `ClassHashNotFound`.
pub fn get_class<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
    class_hash: FieldElement,
) -> RpcResult<ContractClass>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let class_hash = Felt252Wrapper(class_hash).into();

    let contract_class = starknet
        .overrides
        .for_block_hash(starknet.client.as_ref(), substrate_block_hash)
        .contract_class_by_class_hash(substrate_block_hash, class_hash)
        .ok_or_else(|| {
            error!("Failed to retrieve contract class from hash '{class_hash}'");
            StarknetRpcApiError::ClassHashNotFound
        })?;

    // Blockifier classes do not store ABI, has to be retrieved separately
    let contract_abi = starknet
        .overrides
        .for_block_hash(starknet.client.as_ref(), substrate_block_hash)
        .contract_abi_by_class_hash(substrate_block_hash, class_hash)
        .ok_or_else(|| {
            error!("Failed to retrieve contract ABI from hash '{class_hash}'");
            StarknetRpcApiError::ClassHashNotFound
        })?;

    // converting from stored Blockifier class to rpc class
    Ok(ContractClassWrapper { contract: contract_class, abi: contract_abi }.try_into().map_err(|e| {
        error!("Failed to convert contract class from hash '{class_hash}' to RPC contract class: {e}");
        StarknetRpcApiError::InternalServerError
    })?)
}
