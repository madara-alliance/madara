use std::time::{Duration, SystemTime};

use jsonrpsee::core::async_trait;

use crate::{errors::ErrorExtWs, versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server, Starknet};

#[async_trait]
impl MadaraStatusRpcApiV0_1_0Server for Starknet {
    async fn ping(&self) -> jsonrpsee::core::RpcResult<u64> {
        Ok(unix_now())
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn shutdown(&self) -> jsonrpsee::core::RpcResult<u64> {
        self.ctx.cancel_global();
        tracing::info!("🔌 Shutting down node...");
        Ok(unix_now())
    }

    async fn pulse(
        &self,
        subscription_sink: jsonrpsee::PendingSubscriptionSink,
    ) -> jsonrpsee::core::SubscriptionResult {
        let sink =
            subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;

        while !self.ctx.is_cancelled() {
            let now = unix_now();
            let msg = jsonrpsee::SubscriptionMessage::from_json(&now)
                .or_else_internal_server_error(|| format!("Failed to create response message at unix time {now}"))?;
            sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")?;

            tokio::time::sleep(Duration::from_secs(10)).await;
        }

        Ok(())
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
}
