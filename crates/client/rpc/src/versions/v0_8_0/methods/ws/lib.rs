use crate::{
    errors::{StarknetWsApiError, WsResult},
    versions::v0_8_0::StarknetWsRpcApiV0_8_0Server,
};

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_0Server for crate::Starknet {
    async fn foo(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        block_id: starknet_core::types::BlockId,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;

        let msg = jsonrpsee::SubscriptionMessage::from(format!("{block_id:?}"));
        sink.send(msg).await?;

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        #[derive(serde::Serialize)]
        struct StarknetSubscriptionError<'a> {
            code: u32,
            message: &'a str,
        }

        let err = serde_json::to_string(&StarknetSubscriptionError {
            code: 68,
            message: "Cannot go back more than 1024 blocks",
        })
        .unwrap();

        return Err(err.into());
    }

    async fn subscribe_new_heads(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
        block_id: starknet_core::types::BlockId,
    ) -> WsResult {
        WsResult::Err(StarknetWsApiError::TooManyBlocksBack)
    }
}
