use super::AddTransactionProvider;
use crate::{bail_internal_server_error, errors::StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_db::mempool_db::SerializedMempoolTx;
use mc_gateway_client::GatewayProvider;
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_gateway::error::SequencerError;
use mp_rpc::{
    AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
    ClassAndTxnHash, ContractAndTxnHash,
};
use mp_transactions::BroadcastedDeclareTransactionV0;

pub struct ForwardToProvider {
    provider: GatewayProvider,
}

impl ForwardToProvider {
    pub fn new(provider: GatewayProvider) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl AddTransactionProvider for ForwardToProvider {
    async fn add_declare_v0_transaction(
        &self,
        _declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<ClassAndTxnHash> {
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }
    async fn add_declare_transaction(&self, declare_transaction: BroadcastedDeclareTxn) -> RpcResult<ClassAndTxnHash> {
        let sequencer_response = match self
            .provider
            .add_declare_transaction(declare_transaction.try_into().map_err(StarknetRpcApiError::from)?)
            .await
        {
            Ok(response) => response,
            Err(SequencerError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add declare transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTxn,
    ) -> RpcResult<ContractAndTxnHash> {
        let sequencer_response = match self
            .provider
            .add_deploy_account_transaction(deploy_account_transaction.try_into().map_err(StarknetRpcApiError::from)?)
            .await
        {
            Ok(response) => response,
            Err(SequencerError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add deploy account transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTxn,
    ) -> RpcResult<AddInvokeTransactionResult> {
        let sequencer_response = match self
            .provider
            .add_invoke_transaction(invoke_transaction.try_into().map_err(StarknetRpcApiError::from)?)
            .await
        {
            Ok(response) => response,
            Err(SequencerError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }

    async fn add_trusted_validated_transaction(
        &self,
        tx_hash: Felt,
        tx: SerializedMempoolTx,
        converted_class: Option<ConvertedClass>,
    ) -> RpcResult<()> {
        match self.provider.add_verified_transaction(tx_hash, tx, converted_class).await {
            Ok(response) => response,
            Err(SequencerError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
        }
        Ok(())
    }
}
