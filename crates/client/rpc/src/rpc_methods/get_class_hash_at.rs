use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
pub use mc_rpc_core::{Felt, GetClassHashAtServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{BlockId, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> GetClassHashAtServer for Starknet<A, B, BE, G, C, P, H>
where
    A: ChainApi<Block = B> + 'static,
    B: BlockT,
    P: TransactionPool<Block = B> + 'static,
    BE: Backend<B> + 'static,
    C: HeaderBackend<B> + BlockBackend<B> + StorageProvider<B, BE> + 'static,
    C: ProvideRuntimeApi<B>,
    C::Api: StarknetRuntimeApi<B> + ConvertTransactionRuntimeApi<B>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag
    /// * `contract_address` - The address of the contract whose class hash will be returned
    ///
    /// ### Returns
    ///
    /// * `class_hash` - The class hash of the given contract
    fn get_class_hash_at(&self, block_id: BlockId, contract_address: FieldElement) -> RpcResult<Felt> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let contract_address = Felt252Wrapper(contract_address).into();
        let class_hash = self
            .overrides
            .for_block_hash(self.client.as_ref(), substrate_block_hash)
            .contract_class_hash_by_address(substrate_block_hash, contract_address)
            .ok_or_else(|| {
                error!("Failed to retrieve contract class hash at '{contract_address:?}'");
                StarknetRpcApiError::ContractNotFound
            })?;

        Ok(Felt(Felt252Wrapper::from(class_hash).into()))
    }
}
