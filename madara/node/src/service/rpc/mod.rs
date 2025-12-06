use self::server::rpc_api_build;
use crate::{cli::RpcParams, submit_tx::MakeSubmitTransactionSwitch};
use jsonrpsee::server::ServerHandle;
use mc_block_production::BlockProductionHandle;
use mc_db::MadaraBackend;
use mc_rpc::{rpc_api_admin, rpc_api_user, Starknet};
use metrics::RpcMetrics;
use mp_chain_config::RpcVersion;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use server::{start_server, ServerConfig};
use std::sync::Arc;

mod metrics;
mod middleware;
mod server;

#[derive(Clone)]
pub enum RpcType {
    User,
    Admin,
}

pub struct RpcService {
    config: RpcParams,
    backend: Arc<MadaraBackend>,
    submit_tx_provider: MakeSubmitTransactionSwitch,
    server_handle: Option<ServerHandle>,
    rpc_type: RpcType,
    block_prod_handle: Option<BlockProductionHandle>,
    charge_fee: bool,
}

impl RpcService {
    pub fn user(
        config: RpcParams,
        backend: Arc<MadaraBackend>,
        submit_tx_provider: MakeSubmitTransactionSwitch,
        charge_fee: bool,
    ) -> Self {
        Self {
            config,
            backend,
            submit_tx_provider,
            server_handle: None,
            rpc_type: RpcType::User,
            block_prod_handle: None,
            charge_fee,
        }
    }

    pub fn admin(
        config: RpcParams,
        backend: Arc<MadaraBackend>,
        submit_tx_provider: MakeSubmitTransactionSwitch,
        block_prod_handle: BlockProductionHandle,
        charge_fee: bool,
    ) -> Self {
        Self {
            config,
            backend,
            submit_tx_provider,
            server_handle: None,
            rpc_type: RpcType::Admin,
            block_prod_handle: Some(block_prod_handle),
            charge_fee,
        }
    }
}

#[async_trait::async_trait]
impl Service for RpcService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let config = self.config.clone();
        let backend = Arc::clone(&self.backend);
        let submit_tx_provider = self.submit_tx_provider.clone();
        let rpc_type = self.rpc_type.clone();

        let (stop_handle, server_handle) = jsonrpsee::server::stop_channel();

        self.server_handle = Some(server_handle);
        let block_prod_handle = self.block_prod_handle.clone();

        let pre_v0_9_preconfirmed_as_pending = self.config.rpc_pre_v0_9_preconfirmed_as_pending;
        let rpc_unsafe_enabled = self.config.rpc_unsafe;
        let charge_fee = self.charge_fee;

        runner.service_loop(move |ctx| async move {
            let submit_tx = Arc::new(submit_tx_provider.make(ctx.clone()));

            let mut starknet = Starknet::new(
                backend.clone(),
                submit_tx,
                config.storage_proof_config(),
                block_prod_handle,
                ctx.clone(),
            );
            starknet.set_pre_v0_9_preconfirmed_as_pending(pre_v0_9_preconfirmed_as_pending);
            starknet.set_rpc_unsafe_enabled(rpc_unsafe_enabled);
            starknet.set_charge_fee(charge_fee);

            let metrics = RpcMetrics::register()?;

            let server_config = {
                let (name, addr, api_rpc, rpc_version_default, supported_versions) = match rpc_type {
                    RpcType::User => (
                        "JSON-RPC".to_string(),
                        config.addr_user(),
                        rpc_api_user(&starknet)?,
                        mp_chain_config::RpcVersion::RPC_VERSION_LATEST,
                        vec![
                            RpcVersion::RPC_VERSION_0_7_1,
                            RpcVersion::RPC_VERSION_0_8_1,
                            RpcVersion::RPC_VERSION_0_9_0,
                        ],
                    ),
                    RpcType::Admin => (
                        "JSON-RPC (Admin)".to_string(),
                        config.addr_admin(),
                        rpc_api_admin(&starknet)?,
                        mp_chain_config::RpcVersion::RPC_VERSION_LATEST_ADMIN,
                        vec![RpcVersion::RPC_VERSION_ADMIN_0_1_0],
                    ),
                };
                let methods = rpc_api_build("rpc", api_rpc).into();

                ServerConfig {
                    name,
                    addr,
                    batch_config: config.batch_config(),
                    max_connections: config.rpc_max_connections,
                    max_payload_in_mib: config.rpc_max_request_size,
                    max_payload_out_mib: config.rpc_max_response_size,
                    max_subs_per_conn: config.rpc_max_subscriptions_per_connection,
                    message_buffer_capacity: config.rpc_message_buffer_capacity_per_connection,
                    methods,
                    metrics,
                    cors: config.cors(),
                    rpc_version_default,
                    supported_versions,
                }
            };

            start_server(server_config, ctx.clone(), stop_handle, Arc::new(starknet)).await?;

            anyhow::Ok(())
        });

        anyhow::Ok(())
    }
}

impl ServiceId for RpcService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        match self.rpc_type {
            RpcType::User => MadaraServiceId::RpcUser.svc_id(),
            RpcType::Admin => MadaraServiceId::RpcAdmin.svc_id(),
        }
    }
}
