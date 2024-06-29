use starknet_core::types::{BroadcastedInvokeTransaction, InvokeTransactionResult};
use starknet_providers::{Provider, ProviderError};

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::{bail_internal_server_error, Starknet};

/// Add an Invoke Transaction to invoke a contract function
///
/// # Arguments
///
/// * `invoke tx` - <https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/#invoke_transaction>
///
/// # Returns
///
/// * `transaction_hash` - transaction hash corresponding to the invocation
pub async fn add_invoke_transaction(
    starknet: &Starknet,
    invoke_transaction: BroadcastedInvokeTransaction,
) -> StarknetRpcResult<InvokeTransactionResult> {
    let sequencer = starknet.sequencer_provider();

    let sequencer_response = match sequencer.add_invoke_transaction(invoke_transaction).await {
        Ok(response) => response,
        Err(ProviderError::StarknetError(e)) => {
            return Err(StarknetRpcApiError::from(e));
        }
        Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
    };

    Ok(sequencer_response)
}
