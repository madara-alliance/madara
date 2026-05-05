pub async fn starknet_unsubscribe(
    starknet: &crate::Starknet,
    subscription_id: String,
) -> crate::StarknetRpcResult<bool> {
    let subscription_id =
        subscription_id.parse::<u64>().map_err(|_| crate::StarknetRpcApiError::InvalidSubscriptionId)?;

    if starknet.ws_handles.subscription_close(subscription_id).await {
        Ok(true)
    } else {
        Err(crate::StarknetRpcApiError::InvalidSubscriptionId)
    }
}
