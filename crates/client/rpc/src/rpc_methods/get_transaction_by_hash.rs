use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
use mc_rpc_core::ChainIdServer;
pub use mc_rpc_core::{Felt, GetTransactionByHashServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
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
use starknet_core::types::{FieldElement, Transaction};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> GetTransactionByHashServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Get the details and status of a submitted transaction.
    ///
    /// This function retrieves the detailed information and status of a transaction identified by
    /// its hash. The transaction hash uniquely identifies a specific transaction that has been
    /// submitted to the StarkNet network.
    ///
    /// ### Arguments
    ///
    /// * `transaction_hash` - The hash of the requested transaction. This parameter specifies the
    ///   transaction for which details and status are requested.
    ///
    /// ### Returns
    ///
    /// Returns information about the requested transaction, including its status, sender,
    /// recipient, and other transaction details. The information is encapsulated in a `Transaction`
    /// type, which is a combination of the `TXN` schema and additional properties, such as the
    /// `transaction_hash`. In case the specified transaction hash is not found, returns a
    /// `StarknetRpcApiError` with `TXN_HASH_NOT_FOUND`.
    ///
    /// ### Errors
    ///
    /// The function may return one of the following errors if encountered:
    /// - `PAGE_SIZE_TOO_BIG` if the requested page size exceeds the allowed limit.
    /// - `INVALID_CONTINUATION_TOKEN` if the provided continuation token is invalid or expired.
    /// - `BLOCK_NOT_FOUND` if the specified block is not found.
    /// - `TOO_MANY_KEYS_IN_FILTER` if there are too many keys in the filter, which may exceed the
    ///   system's capacity.
    fn get_transaction_by_hash(&self, transaction_hash: FieldElement) -> RpcResult<Transaction> {
        let substrate_block_hash_from_db = self
            .backend
            .mapping()
            .block_hash_from_transaction_hash(Felt252Wrapper::from(transaction_hash).into())
            .map_err(|e| {
                error!("Failed to get transaction's substrate block hash from mapping_db: {e}");
                StarknetRpcApiError::TxnHashNotFound
            })?;

        let substrate_block_hash = match substrate_block_hash_from_db {
            Some(block_hash) => block_hash,
            None => return Err(StarknetRpcApiError::TxnHashNotFound.into()),
        };

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash)?;

        let chain_id = self.chain_id()?.0.into();

        let find_tx =
            if let Some(tx_hashes) = self.get_cached_transaction_hashes(starknet_block.header().hash::<H>().into()) {
                tx_hashes
                    .into_iter()
                    .zip(starknet_block.transactions())
                    .find(|(tx_hash, _)| *tx_hash == Felt252Wrapper(transaction_hash).into())
                    .map(|(_, tx)| to_starknet_core_tx(tx.clone(), transaction_hash))
            } else {
                starknet_block
                    .transactions()
                    .iter()
                    .find(|tx| {
                        tx.compute_hash::<H>(chain_id, false, Some(starknet_block.header().block_number)).0
                            == transaction_hash
                    })
                    .map(|tx| to_starknet_core_tx(tx.clone(), transaction_hash))
            };

        find_tx.ok_or(StarknetRpcApiError::TxnHashNotFound.into())
    }
}
