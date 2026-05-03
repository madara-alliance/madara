use crate::errors::StarknetRpcResult;
use crate::Starknet;

/// Unsubscribes from a WebSocket subscription by ID.
pub async fn starknet_unsubscribe(_starknet: &Starknet, _subscription_id: u64) -> StarknetRpcResult<bool> {
    // FIXME(subscriptions): Implement proper unsubscription once subscriptions are working.
    // For now, return true to indicate successful unsubscription.
    Ok(true)
}
