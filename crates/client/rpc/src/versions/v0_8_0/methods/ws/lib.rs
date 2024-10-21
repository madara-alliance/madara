use crate::versions::v0_8_0::StarknetWsRpcApiV0_8_0Server;

#[jsonrpsee::core::async_trait]
impl StarknetWsRpcApiV0_8_0Server for crate::Starknet {
    async fn foo(&self, pending: jsonrpsee::PendingSubscriptionSink) -> jsonrpsee::core::SubscriptionResult {
        let sink = pending.accept().await?;

        loop {
            let msg = jsonrpsee::SubscriptionMessage::from("Hello");
            sink.send(msg).await?;

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}
