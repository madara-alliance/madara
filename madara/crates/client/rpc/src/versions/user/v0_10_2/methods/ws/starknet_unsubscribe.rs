pub async fn starknet_unsubscribe(
    starknet: &crate::Starknet,
    subscription_id: String,
) -> crate::StarknetRpcResult<bool> {
    if starknet.ws_handles.subscription_close(&subscription_id).await {
        Ok(true)
    } else {
        Err(crate::StarknetRpcApiError::InvalidSubscriptionId)
    }
}
