use blockifier::transaction::errors::TransactionExecutionError;
use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BlockId, BroadcastedTransaction, Felt, SimulatedTransaction, SimulationFlag, TransactionTraceWithHash,
};

use super::simulate_transactions::simulate_transactions;
use super::trace_block_transactions::trace_block_transactions;
use super::trace_transaction::trace_transaction;
use crate::errors::StarknetRpcApiError;
use crate::{Starknet, StarknetTraceRpcApiServer};

#[async_trait]
impl StarknetTraceRpcApiServer for Starknet {
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

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TransactionTraceWithHash> {
        trace_transaction(self, transaction_hash).await
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConvertCallInfoToExecuteInvocationError {
    #[error("One of the simulated transaction failed")]
    TransactionExecutionFailed,
    #[error(transparent)]
    GetFunctionInvocation(#[from] TryFuntionInvocationFromCallInfoError),
    #[error("Missing FunctionInvocation")]
    MissingFunctionInvocation,
}

impl From<ConvertCallInfoToExecuteInvocationError> for StarknetRpcApiError {
    fn from(err: ConvertCallInfoToExecuteInvocationError) -> Self {
        match err {
            ConvertCallInfoToExecuteInvocationError::TransactionExecutionFailed => StarknetRpcApiError::ContractError,
            ConvertCallInfoToExecuteInvocationError::GetFunctionInvocation(_) => {
                StarknetRpcApiError::InternalServerError
            }
            ConvertCallInfoToExecuteInvocationError::MissingFunctionInvocation => StarknetRpcApiError::ContractError,
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
