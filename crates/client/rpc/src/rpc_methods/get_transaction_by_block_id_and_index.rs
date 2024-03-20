use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::CallError;
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
use mc_rpc_core::ChainIdServer;
pub use mc_rpc_core::{
    Felt, GetTransactionByBlockIdAndIndexServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer,
};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{BlockId, FieldElement, Transaction};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> GetTransactionByBlockIdAndIndexServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Get the details of a transaction by a given block id and index.
    ///
    /// This function fetches the details of a specific transaction in the StarkNet network by
    /// identifying it through its block and position (index) within that block. If no transaction
    /// is found at the specified index, null is returned.
    ///
    /// ### Arguments
    ///
    /// * `block_id` - The hash of the requested block, or number (height) of the requested block,
    ///   or a block tag. This parameter is used to specify the block in which the transaction is
    ///   located.
    /// * `index` - An integer representing the index in the block where the transaction is expected
    ///   to be found. The index starts from 0 and increases sequentially for each transaction in
    ///   the block.
    ///
    /// ### Returns
    ///
    /// Returns the details of the transaction if found, including the transaction hash. The
    /// transaction details are returned as a type conforming to the StarkNet protocol. In case of
    /// errors like `BLOCK_NOT_FOUND` or `INVALID_TXN_INDEX`, returns a `StarknetRpcApiError`
    /// indicating the specific issue.
    fn get_transaction_by_block_id_and_index(&self, block_id: BlockId, index: u64) -> RpcResult<Transaction> {
        let substrate_block_hash = self.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
            error!("'{e}'");
            StarknetRpcApiError::BlockNotFound
        })?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash)?;

        let transaction =
            starknet_block.transactions().get(index as usize).ok_or(StarknetRpcApiError::InvalidTxnIndex)?;
        let chain_id = self.chain_id()?;

        let opt_cached_transaction_hashes =
            self.get_cached_transaction_hashes(starknet_block.header().hash::<H>().into());

        let transaction_hash = if let Some(cached_tx_hashes) = opt_cached_transaction_hashes {
            cached_tx_hashes.get(index as usize).map(|&fe| FieldElement::from(Felt252Wrapper::from(fe))).ok_or(
                CallError::Failed(anyhow::anyhow!(
                    "Number of cached tx hashes does not match the number of transactions in block with id {:?}",
                    block_id
                )),
            )?
        } else {
            transaction.compute_hash::<H>(chain_id.0.into(), false, Some(starknet_block.header().block_number)).0
        };

        Ok(to_starknet_core_tx(transaction.clone(), transaction_hash))
    }
}
