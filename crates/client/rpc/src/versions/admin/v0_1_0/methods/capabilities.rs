use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::MadaraCapability;

use crate::{versions::admin::v0_1_0::MadaraCapabilitiesRpcApiV0_1_0Server, Starknet};

const RESTART_INTERVAL: Duration = Duration::from_secs(5);

#[async_trait]
impl MadaraCapabilitiesRpcApiV0_1_0Server for Starknet {
    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_disable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping RPC service...");
        Ok(self.ctx.capabilities_remove(MadaraCapability::Rpc))
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_enable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Starting RPC service...");
        Ok(self.ctx.capabilities_add(MadaraCapability::Rpc))
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_restart(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Restarting RPC service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::Rpc);
        tokio::time::sleep(RESTART_INTERVAL).await;
        self.ctx.capabilities_add(MadaraCapability::Rpc);

        tracing::info!("🔌 Restart complete (Rpc)");

        return Ok(res);
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_disable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::L1Sync)
            | self.ctx.capabilities_remove(MadaraCapability::L2Sync);

        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_enable(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Starting Sync service...");

        let res =
            self.ctx.capabilities_add(MadaraCapability::L1Sync) | self.ctx.capabilities_add(MadaraCapability::L2Sync);

        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_restart(&self) -> RpcResult<bool> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.capabilities_remove(MadaraCapability::L1Sync)
            | self.ctx.capabilities_remove(MadaraCapability::L2Sync);

        tokio::time::sleep(RESTART_INTERVAL).await;

        self.ctx.capabilities_add(MadaraCapability::L1Sync);
        self.ctx.capabilities_add(MadaraCapability::L2Sync);

        tracing::info!("🔌 Restart complete (Sync)");

        Ok(res)
    }
}
