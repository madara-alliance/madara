use crate::{versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use mp_rpc::v0_9_0::{
    BlockId, BlockTag, BroadcastedTxn, SimulateTransactionsResult, SimulationFlag, TraceBlockTransactionsResult,
    TraceTransactionResult,
};
use simulate_transactions::simulate_transactions;
use starknet_types_core::felt::Felt;
use trace_block_transactions::trace_block_transactions;
use trace_transaction::trace_transaction;

pub(crate) mod simulate_transactions;
pub mod trace_block_transactions;
pub(crate) mod trace_transaction;

fn validate_trace_block_transactions_block_id(block_id: &BlockId) -> RpcResult<()> {
    if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
        return Err(ErrorObjectOwned::owned(INVALID_PARAMS_CODE, "Invalid params", None::<()>));
    }
    Ok(())
}

#[async_trait]
impl StarknetTraceRpcApiV0_9_0Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        Ok(simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TraceBlockTransactionsResult>> {
        validate_trace_block_transactions_block_id(&block_id)?;
        Ok(trace_block_transactions(self, block_id).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        Ok(trace_transaction(self, transaction_hash).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_block_transactions_rejects_pre_confirmed_tag() {
        let err = validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::PreConfirmed))
            .expect_err("pre_confirmed should be rejected for traceBlockTransactions");
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
        assert_eq!(err.message(), "Invalid params");
    }

    #[test]
    fn trace_block_transactions_accepts_latest_tag() {
        assert!(validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::Latest)).is_ok());
    }
}
