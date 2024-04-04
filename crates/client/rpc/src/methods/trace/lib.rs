use blockifier::transaction::errors::TransactionExecutionError;
use jsonrpsee::core::{async_trait, RpcResult};
use log::error;
use mc_genesis_data_provider::GenesisProvider;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, SimulatedTransaction, SimulationFlag, TransactionTraceWithHash,
};
use starknet_ff::FieldElement;
use thiserror::Error;

use super::simulate_transactions::*;
use super::trace_implementation::*;
use crate::errors::StarknetRpcApiError;
use crate::{Starknet, StarknetTraceRpcApiServer};

#[async_trait]
impl<A, BE, G, C, P, H> StarknetTraceRpcApiServer for Starknet<A, BE, G, C, P, H>
where
    A: ChainApi<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    G: GenesisProvider + Send + Sync + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    P: TransactionPool<Block = DBlockT> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransaction>> {
        simulate_transactions(self, block_id, transactions, simulation_flags).await
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TransactionTraceWithHash>> {
        trace_block_transactions(self, block_id).await
    }

    async fn trace_transaction(&self, transaction_hash: FieldElement) -> RpcResult<TransactionTraceWithHash> {
        trace_transaction(self, transaction_hash).await
    }
}

#[derive(Error, Debug)]
pub enum ConvertCallInfoToExecuteInvocationError {
    #[error("One of the simulated transaction failed")]
    TransactionExecutionFailed,
    #[error(transparent)]
    GetFunctionInvocation(#[from] TryFuntionInvocationFromCallInfoError),
}

impl From<ConvertCallInfoToExecuteInvocationError> for StarknetRpcApiError {
    fn from(err: ConvertCallInfoToExecuteInvocationError) -> Self {
        match err {
            ConvertCallInfoToExecuteInvocationError::TransactionExecutionFailed => StarknetRpcApiError::ContractError,
            ConvertCallInfoToExecuteInvocationError::GetFunctionInvocation(_) => {
                StarknetRpcApiError::InternalServerError
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum TryFuntionInvocationFromCallInfoError {
    #[error(transparent)]
    TransactionExecution(#[from] TransactionExecutionError),
    #[error("No contract found at the Call contract_address")]
    ContractNotFound,
}
