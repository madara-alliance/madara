use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use mp_utils::service::{MadaraServiceId, ServiceStatus};

use crate::{
    versions::admin::v0_1_0::{MadaraServicesRpcApiV0_1_0Server, ServiceRequest},
    Starknet,
};

const RESTART_INTERVAL: Duration = Duration::from_secs(5);

#[async_trait]
impl MadaraServicesRpcApiV0_1_0Server for Starknet {
    async fn service(&self, svcs: Vec<MadaraServiceId>, status: ServiceRequest) -> RpcResult<Vec<ServiceStatus>> {
        if svcs.is_empty() {
            Err(jsonrpsee::types::ErrorObject::owned(
                jsonrpsee::types::ErrorCode::InvalidParams.code(),
                "You must provide at least one service to toggle",
                Some(()),
            ))
        } else {
            match status {
                ServiceRequest::Start => {
                    Ok(futures::future::join_all(svcs.iter().map(|svc| self.ctx.service_activate(*svc))).await)
                }
                ServiceRequest::Stop => {
                    Ok(futures::future::join_all(svcs.iter().map(|svc| self.ctx.service_deactivate(*svc))).await)
                }
                ServiceRequest::Restart => {
                    let res =
                        Ok(futures::future::join_all(svcs.iter().map(|svc| self.ctx.service_activate(*svc))).await);
                    tokio::time::sleep(RESTART_INTERVAL).await;
                    for svc in svcs {
                        self.ctx.service_activate(svc).await;
                    }

                    res
                }
            }
        }
    }
}
