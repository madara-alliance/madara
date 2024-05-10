use jsonrpsee::core::RpcResult;
use mc_sync::utility::get_config;
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BroadcastedDeployAccountTransaction, DeployAccountTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

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
pub async fn add_deploy_account_transaction<BE, C, H>(
    _starknet: &Starknet<BE, C, H>,
    deploy_account_transaction: BroadcastedDeployAccountTransaction,
) -> RpcResult<DeployAccountTransactionResult>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    H: HasherT + Send + Sync + 'static,
{
    let config = get_config().ok_or_else(|| {
        log::error!("Failed to get config");
        StarknetRpcApiError::InternalServerError
    })?;
    let sequencer = SequencerGatewayProvider::new(config.feeder_gateway, config.gateway, config.chain_id);

    let sequencer_response = match sequencer.add_deploy_account_transaction(deploy_account_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e).into());
        }
        Err(e) => {
            log::error!("Failed to add invoke transaction to sequencer: {e}");
            return Err(StarknetRpcApiError::InternalServerError.into());
        }
    };

    Ok(sequencer_response)
}
