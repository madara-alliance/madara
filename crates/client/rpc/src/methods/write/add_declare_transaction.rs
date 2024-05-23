use jsonrpsee::core::RpcResult;
use mc_sync::utility::{chain_id, feeder_gateway, gateway};
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BroadcastedDeclareTransaction, DeclareTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Submit a new declare transaction to be added to the chain
///
/// # Arguments
///
/// * `declare_transaction` - the declare transaction to be added to the chain
///
/// # Returns
///
/// * `declare_transaction_result` - the result of the declare transaction
pub async fn add_declare_transaction<BE, C, H>(
    _starknet: &Starknet<BE, C, H>,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let sequencer = SequencerGatewayProvider::new(feeder_gateway(), gateway(), chain_id());

    let sequencer_response = match sequencer.add_declare_transaction(declare_transaction).await {
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
