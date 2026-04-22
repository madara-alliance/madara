use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::{MadaraServiceId, MadaraServiceStatus, ServiceContext};

use crate::{
    versions::admin::v0_1_0::{MadaraServicesRpcApiV0_1_0Server, ServiceRequest, ServiceStatusInfo},
    Starknet,
};

const RESTART_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_EXTERNALLY_CONTROLLABLE_SERVICES: [MadaraServiceId; 6] = [
    MadaraServiceId::L1Sync,
    MadaraServiceId::L2Sync,
    MadaraServiceId::BlockProduction,
    MadaraServiceId::RpcUser,
    MadaraServiceId::Gateway,
    MadaraServiceId::Mempool,
];

#[async_trait]
impl MadaraServicesRpcApiV0_1_0Server for Starknet {
    async fn service(&self, service: Vec<MadaraServiceId>, status: ServiceRequest) -> RpcResult<MadaraServiceStatus> {
        if service.is_empty() {
            Err(jsonrpsee::types::ErrorObject::owned(
                jsonrpsee::types::ErrorCode::InvalidParams.code(),
                "You must provide at least one service to toggle",
                Some(()),
            ))
        } else {
            ensure_services_are_externally_controllable(&service)?;
            match status {
                ServiceRequest::Start => service_start(&self.ctx, &service),
                ServiceRequest::Stop => service_stop(&self.ctx, &service),
                ServiceRequest::Restart => service_restart(&self.ctx, &service).await,
            }
        }
    }

    async fn service_status(&self, service: Vec<MadaraServiceId>) -> RpcResult<Vec<ServiceStatusInfo>> {
        let services = if service.is_empty() {
            // Note: a few services are intentionally not externally controllable
            // (`Monitor`, `Database`, `RpcAdmin`, `Telemetry`) and are not
            // included in the default admin-RPC list.
            DEFAULT_EXTERNALLY_CONTROLLABLE_SERVICES.to_vec()
        } else {
            service
        };

        Ok(services
            .into_iter()
            .map(|svc| ServiceStatusInfo {
                service: svc,
                requested: self.ctx.service_status_requested(svc),
                actual: self.ctx.service_status_actual(svc),
            })
            .collect())
    }
}

fn ensure_services_are_externally_controllable(svcs: &[MadaraServiceId]) -> RpcResult<()> {
    if svcs.contains(&MadaraServiceId::Telemetry) {
        return Err(jsonrpsee::types::ErrorObject::owned(
            jsonrpsee::types::ErrorCode::InvalidParams.code(),
            "Telemetry is process-global and cannot be started, stopped, or restarted via admin RPC",
            Some(()),
        ));
    }

    Ok(())
}

fn service_start(ctx: &ServiceContext, svcs: &[MadaraServiceId]) -> RpcResult<MadaraServiceStatus> {
    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        tracing::info!("🔌 Starting {} service...", svc);
        status |= ctx.service_add(*svc);
    }

    Ok(status)
}

fn service_stop(ctx: &ServiceContext, svcs: &[MadaraServiceId]) -> RpcResult<MadaraServiceStatus> {
    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        tracing::info!("🔌 Stopping {} service...", svc);
        status |= ctx.service_remove(*svc);
    }

    Ok(status)
}

async fn service_restart(ctx: &ServiceContext, svcs: &[MadaraServiceId]) -> RpcResult<MadaraServiceStatus> {
    let mut status = MadaraServiceStatus::Off;
    for svc in svcs {
        tracing::info!("🔌 Restarting {} service...", svc);
        status |= ctx.service_remove(*svc);
    }

    tokio::time::sleep(RESTART_INTERVAL).await;

    for svc in svcs {
        ctx.service_add(*svc);
        tracing::info!("🔌 Restart {} complete", svc);
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_service_is_not_externally_controllable() {
        let err = ensure_services_are_externally_controllable(&[MadaraServiceId::Telemetry]).unwrap_err();

        assert_eq!(err.code(), jsonrpsee::types::ErrorCode::InvalidParams.code());
    }

    #[test]
    fn non_telemetry_services_remain_externally_controllable() {
        ensure_services_are_externally_controllable(&[MadaraServiceId::Gateway, MadaraServiceId::Mempool]).unwrap();
    }
}
