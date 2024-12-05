use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus, ServiceContext};

use crate::{versions::admin::v0_1_0::MadaraServicesRpcApiV0_1_0Server, Starknet};

const RESTART_INTERVAL: Duration = Duration::from_secs(5);

#[async_trait]
impl MadaraServicesRpcApiV0_1_0Server for Starknet {
    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_disable(&self) -> RpcResult<MadaraServiceStatus> {
        services_disable(&self.ctx, &[MadaraServiceId::L1Sync, MadaraServiceId::L2Sync], "Sync")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_enable(&self) -> RpcResult<MadaraServiceStatus> {
        services_enable(&self.ctx, &[MadaraServiceId::L1Sync, MadaraServiceId::L2Sync], "Sync")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_sync_restart(&self) -> RpcResult<MadaraServiceStatus> {
        services_restart(&self.ctx, &[MadaraServiceId::L1Sync, MadaraServiceId::L2Sync], "Sync").await
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_block_production_disable(&self) -> RpcResult<MadaraServiceStatus> {
        services_disable(&self.ctx, &[MadaraServiceId::BlockProduction], "Block production")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_block_production_enable(&self) -> RpcResult<MadaraServiceStatus> {
        services_enable(&self.ctx, &[MadaraServiceId::BlockProduction], "Block production")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_block_production_restart(&self) -> RpcResult<MadaraServiceStatus> {
        services_restart(&self.ctx, &[MadaraServiceId::BlockProduction], "Block production").await
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_disable(&self) -> RpcResult<MadaraServiceStatus> {
        services_disable(&self.ctx, &[MadaraServiceId::RpcUser], "Rpc")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_enable(&self) -> RpcResult<MadaraServiceStatus> {
        services_enable(&self.ctx, &[MadaraServiceId::RpcUser], "Rpc")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_rpc_restart(&self) -> RpcResult<MadaraServiceStatus> {
        services_restart(&self.ctx, &[MadaraServiceId::RpcUser], "Rpc").await
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_gateway_disable(&self) -> RpcResult<MadaraServiceStatus> {
        services_disable(&self.ctx, &[MadaraServiceId::Gateway], "Gateway")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_gateway_enable(&self) -> RpcResult<MadaraServiceStatus> {
        services_enable(&self.ctx, &[MadaraServiceId::Gateway], "Gateway")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_gateway_restart(&self) -> RpcResult<MadaraServiceStatus> {
        services_restart(&self.ctx, &[MadaraServiceId::Gateway], "Gateway").await
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_telemetry_disable(&self) -> RpcResult<MadaraServiceStatus> {
        services_disable(&self.ctx, &[MadaraServiceId::Telemetry], "Telemetry")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_telemetry_enable(&self) -> RpcResult<MadaraServiceStatus> {
        services_enable(&self.ctx, &[MadaraServiceId::Telemetry], "Telemetry")
    }

    #[tracing::instrument(skip(self), fields(module = "Admin"))]
    async fn service_telemetry_restart(&self) -> RpcResult<MadaraServiceStatus> {
        services_restart(&self.ctx, &[MadaraServiceId::Telemetry], "Telemetry").await
    }
}

fn services_disable(ctx: &ServiceContext, svcs: &[MadaraServiceId], name: &str) -> RpcResult<MadaraServiceStatus> {
    tracing::info!("🔌 Stopping {name} service...");

    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        status |= ctx.service_remove(*svc);
    }

    Ok(status)
}

fn services_enable(ctx: &ServiceContext, svcs: &[MadaraServiceId], name: &str) -> RpcResult<MadaraServiceStatus> {
    tracing::info!("🔌 Starting {name} service...");

    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        status |= ctx.service_add(*svc);
    }

    Ok(status)
}

async fn services_restart(
    ctx: &ServiceContext,
    svcs: &[MadaraServiceId],
    name: &str,
) -> RpcResult<MadaraServiceStatus> {
    tracing::info!("🔌 Restarting {name} service...");

    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        status |= ctx.service_remove(*svc);
    }

    tokio::time::sleep(RESTART_INTERVAL).await;

    for svc in svcs {
        ctx.service_add(*svc);
    }

    tracing::info!("🔌 Restart {name} complete");

    Ok(status)
}
