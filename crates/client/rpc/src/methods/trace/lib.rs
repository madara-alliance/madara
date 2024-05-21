use blockifier::transaction::errors::TransactionExecutionError;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, SimulatedTransaction, SimulationFlag, TransactionTraceWithHash,
};
use starknet_ff::FieldElement;

use super::simulate_transactions::simulate_transactions;
use super::trace_block_transactions::trace_block_transactions;
use super::trace_transaction::trace_transaction;
use crate::errors::StarknetRpcApiError;
use crate::{Starknet, StarknetTraceRpcApiServer};

#[async_trait]
impl<BE, C, H> StarknetTraceRpcApiServer for Starknet<BE, C, H>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
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

#[derive(thiserror::Error, Debug)]
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

#[derive(thiserror::Error, Debug)]
pub enum TryFuntionInvocationFromCallInfoError {
    #[error(transparent)]
    TransactionExecution(#[from] TransactionExecutionError),
    #[error("No contract found at the Call contract_address")]
    ContractNotFound,
}
