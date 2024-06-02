use jsonrpsee::core::RpcResult;
use mc_sync::utility::{chain_id, feeder_gateway, gateway};
use starknet_core::types::{BroadcastedDeclareTransaction, DeclareTransactionResult};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};

use crate::errors::StarknetRpcApiError;
use crate::{bail_internal_server_error, Starknet};

/// Submit a new declare transaction to be added to the chain
///
/// # Arguments
///
/// * `declare_transaction` - the declare transaction to be added to the chain
///
/// # Returns
///
/// * `declare_transaction_result` - the result of the declare transaction
pub async fn add_declare_transaction(
    _starknet: &Starknet,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult> {
    let sequencer = SequencerGatewayProvider::new(feeder_gateway(), gateway(), chain_id());

    let sequencer_response = match sequencer.add_declare_transaction(declare_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e).into());
        }
        Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
    };

    Ok(sequencer_response)
}
