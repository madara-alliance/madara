use jsonrpsee::core::RpcResult;
use mc_sync::utility::{chain_id, feeder_gateway, gateway};
use starknet_core::types::{BroadcastedDeployAccountTransaction, DeployAccountTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
use crate::{bail_internal_server_error, Starknet};

/// Add an Deploy Account Transaction
///
/// # Arguments
///
/// * `deploy account transaction` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#deploy_account_transaction>
///
/// # Returns
///
/// * `transaction_hash` - transaction hash corresponding to the invocation
/// * `contract_address` - address of the deployed contract account
pub async fn add_deploy_account_transaction(
    _starknet: &Starknet,
    deploy_account_transaction: BroadcastedDeployAccountTransaction,
) -> RpcResult<DeployAccountTransactionResult> {
    let sequencer = SequencerGatewayProvider::new(feeder_gateway(), gateway(), chain_id());

    let sequencer_response = match sequencer.add_deploy_account_transaction(deploy_account_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e).into());
        }
        Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
    };

    Ok(sequencer_response)
}
