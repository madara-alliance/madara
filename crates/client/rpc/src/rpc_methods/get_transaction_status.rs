use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
pub use mc_rpc_core::utils::*;
use mc_rpc_core::GetTransactionStatusServer;
pub use mc_rpc_core::{
    BlockNumberServer, Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer,
};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_transactions::TransactionStatus;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{FieldElement, TransactionExecutionStatus, TransactionFinalityStatus};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

#[async_trait]
#[allow(unused_variables)]
impl<A, B, BE, G, C, P, H> GetTransactionStatusServer for Starknet<A, B, BE, G, C, P, H>
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
    /// Gets the Transaction Status, Including Mempool Status and Execution Details
    ///
    /// This method retrieves the status of a specified transaction. It provides information on
    /// whether the transaction is still in the mempool, has been executed, or dropped from the
    /// mempool. The status includes both finality status and execution status of the
    /// transaction.
    ///
    /// ### Arguments
    ///
    /// * `transaction_hash` - The hash of the transaction for which the status is requested.
    ///
    /// ### Returns
    ///
    /// * `transaction_status` - An object containing the transaction status details:
    ///   - `finality_status`: The finality status of the transaction, indicating whether it is
    ///     confirmed, pending, or rejected.
    ///   - `execution_status`: The execution status of the transaction, providing details on the
    ///     execution outcome if the transaction has been processed.
    fn get_transaction_status(&self, transaction_hash: FieldElement) -> RpcResult<TransactionStatus> {
        let substrate_block_hash = self
            .backend
            .mapping()
            .block_hash_from_transaction_hash(Felt252Wrapper(transaction_hash).into())
            .map_err(|e| {
                error!("Failed to get transaction's substrate block hash from mapping_db: {e}");
                StarknetRpcApiError::TxnHashNotFound
            })?
            .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

        let starknet_block = get_block_by_block_hash(self.client.as_ref(), substrate_block_hash)?;

        let chain_id = self.chain_id()?.0.into();

        let starknet_tx =
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

        let execution_status = {
            let revert_error = self
                .client
                .runtime_api()
                .get_tx_execution_outcome(substrate_block_hash, Felt252Wrapper(transaction_hash).into())
                .map_err(|e| {
                    error!(
                        "Failed to get transaction execution outcome. Substrate block hash: {substrate_block_hash}, \
                         transaction hash: {transaction_hash}, error: {e}"
                    );
                    StarknetRpcApiError::InternalServerError
                })?;

            if revert_error.is_none() {
                TransactionExecutionStatus::Succeeded
            } else {
                TransactionExecutionStatus::Reverted
            }
        };

        Ok(TransactionStatus { finality_status: TransactionFinalityStatus::AcceptedOnL2, execution_status })
    }
}
