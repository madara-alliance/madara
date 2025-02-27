use jsonrpsee::core::{async_trait, RpcResult};
use mp_transactions::BroadcastedDeclareTransactionV0;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::ClassAndTxnHash;

use crate::{versions::admin::v0_1_0::MadaraWriteRpcApiV0_1_0Server, Starknet};

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
        declare_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash<Felt>> {
        self.add_transaction_provider.add_declare_v0_transaction(declare_transaction).await
    }
}
