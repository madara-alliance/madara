use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus};

use crate::{versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server, Starknet};

const RESTART_INTERVAL: Duration = Duration::from_secs(5);

#[async_trait]
impl MadaraServicesRpcApiV0_1_0Server for Starknet {
    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_disable(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Stopping RPC service...");
        Ok(self.ctx.service_remove(MadaraServiceId::RpcUser))
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_enable(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Starting RPC service...");
        Ok(self.ctx.service_add(MadaraServiceId::RpcUser))
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_restart(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Restarting RPC service...");

        let res = self.ctx.service_remove(MadaraServiceId::RpcUser);
        tokio::time::sleep(RESTART_INTERVAL).await;
        self.ctx.service_add(MadaraServiceId::RpcUser);

        tracing::info!("🔌 Restart complete (Rpc)");

        return Ok(res);
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_disable(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.service_remove(MadaraServiceId::L1Sync) | self.ctx.service_remove(MadaraServiceId::L2Sync);

        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_enable(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Starting Sync service...");

        let res = self.ctx.service_add(MadaraServiceId::L1Sync) | self.ctx.service_add(MadaraServiceId::L2Sync);

        Ok(res)
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_restart(&self) -> RpcResult<MadaraServiceStatus> {
        tracing::info!("🔌 Stopping Sync service...");

        let res = self.ctx.service_remove(MadaraServiceId::L1Sync) | self.ctx.service_remove(MadaraServiceId::L2Sync);

        tokio::time::sleep(RESTART_INTERVAL).await;

        self.ctx.service_add(MadaraServiceId::L1Sync);
        self.ctx.service_add(MadaraServiceId::L2Sync);

        tracing::info!("🔌 Restart complete (Sync)");

        Ok(res)
    }
}
