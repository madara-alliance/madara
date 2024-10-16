use crate::{versions::v0_7_1::MadaraWriteRpcApiV0_7_1Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_transactions::BroadcastedDeclareTransactionV0;
use starknet_core::types::DeclareTransactionResult;

#[async_trait]
impl MadaraWriteRpcApiV0_7_1Server for Starknet {
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
        declare_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<DeclareTransactionResult> {
        Ok(self.add_transaction_provider.add_declare_v0_transaction(declare_transaction).await?)
    }
}
