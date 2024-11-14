use crate::{bail_internal_server_error, errors::StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mc_gateway_provider::GatewayProvider;
use mp_gateway::error::SequencerError;
use mp_transactions::BroadcastedDeclareTransactionV0;
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
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
        _declare_v0_transaction: BroadcastedDeclareTransactionV0,
    ) -> RpcResult<DeclareTransactionResult> {
        Err(StarknetRpcApiError::UnimplementedMethod.into())
    }
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        let sequencer_response = match self.provider.add_declare_transaction(declare_transaction.into()).await {
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
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        let sequencer_response =
            match self.provider.add_deploy_account_transaction(deploy_account_transaction.into()).await {
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
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        let sequencer_response = match self.provider.add_invoke_transaction(invoke_transaction.into()).await {
            Ok(response) => response,
            Err(SequencerError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }
}
