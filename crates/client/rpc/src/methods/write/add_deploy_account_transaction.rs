use jsonrpsee::core::RpcResult;
use starknet_core::types::{BroadcastedDeployAccountTransaction, DeployAccountTransactionResult};
use starknet_providers::{Provider, ProviderError};

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
    starknet: &Starknet,
    deploy_account_transaction: BroadcastedDeployAccountTransaction,
) -> RpcResult<DeployAccountTransactionResult> {
    let sequencer = starknet.sequencer_provider();

    let sequencer_response = match sequencer.add_deploy_account_transaction(deploy_account_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e).into());
        }
        Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
    };

    Ok(sequencer_response)
}
