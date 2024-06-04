use jsonrpsee::core::RpcResult;
use starknet_core::types::{BroadcastedDeclareTransaction, DeclareTransactionResult};
use starknet_providers::{Provider, ProviderError};

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
    starknet: &Starknet,
    declare_transaction: BroadcastedDeclareTransaction,
) -> RpcResult<DeclareTransactionResult> {
    let sequencer = starknet.make_sequencer_provider();

    let sequencer_response = match sequencer.add_declare_transaction(declare_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e).into());
        }
        Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
    };

    Ok(sequencer_response)
}
