use crate::{bail_internal_server_error, errors::StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_gateway_client::GatewayProvider;
use mp_gateway::error::SequencerError;
use mp_rpc::{
    admin::BroadcastedDeclareTxnV0, AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn,
    BroadcastedInvokeTxn, ClassAndTxnHash, ContractAndTxnHash,
};

use super::AddTransactionProvider;

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
        _declare_v0_transaction: BroadcastedDeclareTxnV0,
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
}
