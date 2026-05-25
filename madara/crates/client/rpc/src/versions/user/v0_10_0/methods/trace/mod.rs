use crate::versions::user::v0_9_0::methods::trace::{
    simulate_transactions::simulate_transactions as simulate_transactions_v0_9,
    trace_block_transactions::trace_block_transactions as trace_block_transactions_v0_9,
    trace_transaction::trace_transaction as trace_transaction_v0_9,
};
use crate::{errors::StarknetRpcApiError, versions::user::v0_10_0::StarknetTraceRpcApiV0_10_0Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_0::{
    BlockId, BlockTag, BroadcastedTxn, SimulateTransactionsResult, SimulationFlag, TraceBlockTransactionsResult,
    TraceTransactionResult,
};
use starknet_types_core::felt::Felt;

fn validate_trace_block_transactions_block_id(block_id: &BlockId) -> RpcResult<()> {
    if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
        return Err(StarknetRpcApiError::CallOnPreConfirmed.into());
    }
    Ok(())
}

#[async_trait]
impl StarknetTraceRpcApiV0_10_0Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        let v0_9_results = simulate_transactions_v0_9(self, block_id, transactions, simulation_flags).await?;
        Ok(v0_9_results.into_iter().map(Into::into).collect())
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TraceBlockTransactionsResult>> {
        validate_trace_block_transactions_block_id(&block_id)?;
        let v0_9_results = trace_block_transactions_v0_9(self, block_id).await?;
        Ok(v0_9_results.into_iter().map(Into::into).collect())
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        let v0_9_result = trace_transaction_v0_9(self, transaction_hash).await?;
        Ok(v0_9_result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_block_transactions_rejects_pre_confirmed_tag_with_spec_error() {
        let err = validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::PreConfirmed))
            .expect_err("pre_confirmed should be rejected for traceBlockTransactions");
        assert_eq!(err.code(), 70);
        assert_eq!(err.message(), "This method does not support being called on the pre_confirmed block");
    }

    #[test]
    fn trace_block_transactions_accepts_latest_tag() {
        assert!(validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::Latest)).is_ok());
    }
}
