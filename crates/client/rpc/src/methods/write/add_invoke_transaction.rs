use jsonrpsee::core::RpcResult;
use mc_sync::utility::{chain_id, feeder_gateway, gateway};
use mp_hashers::HasherT;
use mp_types::block::DBlockT;
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BroadcastedInvokeTransaction, InvokeTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Add an Invoke Transaction to invoke a contract function
///
/// # Arguments
///
/// * `invoke tx` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#invoke_transaction>
///
/// # Returns
///
/// * `transaction_hash` - transaction hash corresponding to the invocation
pub async fn add_invoke_transaction<BE, C, H>(
    _starknet: &Starknet<BE, C, H>,
    invoke_transaction: BroadcastedInvokeTransaction,
) -> RpcResult<InvokeTransactionResult>
where
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let sequencer = SequencerGatewayProvider::new(feeder_gateway(), gateway(), chain_id());

    let sequencer_response = match sequencer.add_invoke_transaction(invoke_transaction).await {
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
