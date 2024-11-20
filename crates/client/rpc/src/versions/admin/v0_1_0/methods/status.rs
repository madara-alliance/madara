use std::time::SystemTime;

use jsonrpsee::core::async_trait;

use crate::{versions::admin::v0_1_0::MadaraStatusRpcApiV0_1_0Server, Starknet};

#[async_trait]
impl MadaraStatusRpcApiV0_1_0Server for Starknet {
    /// Stops the node by gracefully shutting down each of its services.
    ///
    /// # Returns
    ///
    /// * Time of shutdown in unix time.
    async fn stop_node(&self) -> jsonrpsee::core::RpcResult<u64> {
        self.cancellation_token().cancel();
        tracing::info!("🔌 Shutting down node...");
        Ok(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs())
    }
}
