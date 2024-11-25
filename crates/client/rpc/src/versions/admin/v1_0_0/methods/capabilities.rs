use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::MadaraCapability;

use crate::{versions::admin::v1_0_0::MadaraCapabilitiesRpcApiV1_0_0Server, Starknet};

#[async_trait]
impl MadaraCapabilitiesRpcApiV1_0_0Server for Starknet {
    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_disable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping RPC service...");
        Ok(self.ctx.capabilities_remove(MadaraCapability::Rpc))
    }

    async fn service_rpc_enable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Starting RPC service...");
        Ok(self.ctx.capabilities_add(MadaraCapability::Rpc))
    }

    async fn service_rpc_restart(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Restarting RPC service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::Rpc);
        tokio::time::sleep(Duration::from_secs(5)).await;
        self.ctx.capabilities_add(MadaraCapability::Rpc);

        tracing::info!("🔌 Restart complete (Rpc)");

        return Ok(res);
    }

    async fn service_sync_disable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::L1Sync)
            | self.ctx.capabilities_remove(MadaraCapability::L2Sync);

        Ok(res)
    }

    async fn service_sync_enable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Starting Sync service...");

        let res =
            self.ctx.capabilities_add(MadaraCapability::L1Sync) | self.ctx.capabilities_add(MadaraCapability::L2Sync);

        Ok(res)
    }

    async fn service_sync_restart(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::L1Sync)
            | self.ctx.capabilities_remove(MadaraCapability::L2Sync);

        tokio::time::sleep(Duration::from_secs(5)).await;

        self.ctx.capabilities_add(MadaraCapability::L1Sync);
        self.ctx.capabilities_add(MadaraCapability::L2Sync);

        tracing::info!("🔌 Restart complete (Sync)");

        Ok(res)
    }
}
