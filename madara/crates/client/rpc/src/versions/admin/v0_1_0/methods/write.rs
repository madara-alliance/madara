use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::MadaraBlockInfo;
use mp_rpc::{admin::BroadcastedDeclareTxnV0, ClassAndTxnHash};

use crate::{versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet, StarknetRpcApiError};

#[async_trait]
impl MadaraWriteRpcApiV0_1_0Server for Starknet {
    /// Submit a new declare transaction to be added to the chain
    ///
    /// # Arguments
    ///
    /// * `declare_v0_transaction` - the declare v0 transaction to be added to the chain
    ///
    /// # Returns
    ///
    /// * `declare_transaction_result` - the result of the declare transaction
    async fn add_declare_v0_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTxnV0,
    ) -> RpcResult<ClassAndTxnHash> {
        self.add_transaction_provider.add_declare_v0_transaction(declare_transaction).await
    }

    /// Ask block producer to close the current pending block
    ///
    /// # Returns
    ///
    /// * `block_info` - the block info of the closed block
    async fn close_block(&self) -> RpcResult<MadaraBlockInfo> {
        let mut rx = self.backend.subscribe_block_info();
        match self.close_block_trigger.as_ref() {
            Some(trigger) => {
                trigger.notify_one();
                rx.recv().await.map_err(|_| StarknetRpcApiError::InternalServerError.into())
            }
            None => Err(StarknetRpcApiError::UnimplementedMethod.into()),
        }
    }
}
