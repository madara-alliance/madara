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
        if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
            return Err(ErrorObjectOwned::owned(INVALID_PARAMS_CODE, "Invalid params", None::<()>));
        }
        Ok(trace_block_transactions(self, block_id).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        Ok(trace_transaction(self, transaction_hash).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;

    #[rstest]
    #[tokio::test]
    async fn test_trace_block_transactions_rejects_pre_confirmed_block_tag(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (_sample_chain, rpc) = sample_chain_for_block_getters;

        let err = StarknetTraceRpcApiV0_9_0Server::trace_block_transactions(&rpc, BlockId::Tag(BlockTag::PreConfirmed))
            .await
            .expect_err("pre_confirmed must be rejected by the RPC layer");

        assert_eq!(err.code(), INVALID_PARAMS_CODE);
        assert_eq!(err.message(), "Invalid params");
    }
}
