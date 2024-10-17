use jsonrpsee::core::async_trait;

use crate::versions::v0_8_1::StarknetWsApiV0_8_0Server;
use crate::Starknet;

#[async_trait]
impl StarknetWsApiV0_8_0Server for Starknet {
    async fn foo(&self, subscription_sink: jsonrpsee::PendingSubscriptionSink) -> jsonrpsee::core::SubscriptionResult {
        let sink = subscription_sink.accept().await?;

        sink.send(jsonrpsee::SubscriptionMessage::from("Hello")).await?;
        sink.send(jsonrpsee::SubscriptionMessage::from("World")).await?;

        Ok(())
    }
}
