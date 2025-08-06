use crate::{versions::user::v0_8_1::StarknetTraceRpcApiV0_8_1Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::BlockId;
use mp_rpc::v0_8_1::{BroadcastedTxn, SimulateTransactionsResult, SimulationFlag};
use simulate_transactions::simulate_transactions;

pub(crate) mod simulate_transactions;

#[async_trait]
impl StarknetTraceRpcApiV0_8_1Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        Ok(simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }
}
