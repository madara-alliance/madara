use std::sync::Arc;

use jsonrpsee::server::ServerHandle;
use tokio::task::JoinSet;

use mc_db::MadaraBackend;
use mc_rpc::{providers::AddTransactionProvider, rpc_api_admin, rpc_api_user, Starknet};
use mp_utils::service::{MadaraService, Service, ServiceContext};

use metrics::RpcMetrics;
use server::{start_server, ServerConfig};

use crate::cli::RpcParams;

use self::server::rpc_api_build;

mod metrics;
mod middleware;
mod server;

pub struct RpcService {
    config: RpcParams,
    backend: Arc<MadaraBackend>,
    add_txs_method_provider: Arc<dyn AddTransactionProvider>,
    server_handle_user: Option<ServerHandle>,
    server_handle_admin: Option<ServerHandle>,
}

impl RpcService {
    pub fn new(
        config: RpcParams,
        backend: Arc<MadaraBackend>,
        add_txs_method_provider: Arc<dyn AddTransactionProvider>,
    ) -> Self {
        Self { config, backend, add_txs_method_provider, server_handle_user: None, server_handle_admin: None }
    }
}

#[async_trait::async_trait]
impl Service for RpcService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>, ctx: ServiceContext) -> anyhow::Result<()> {
        let RpcService { config, backend, add_txs_method_provider, .. } = self;

        let starknet =
            Starknet::new(backend.clone(), add_txs_method_provider.clone(), config.storage_proof_config(), ctx.clone());
        let metrics = RpcMetrics::register()?;

        let server_config_user = if !config.rpc_disable {
            let api_rpc_user = rpc_api_user(&starknet)?;
            let methods_user = rpc_api_build("rpc", api_rpc_user).into();

            Some(ServerConfig {
                name: "JSON-RPC".to_string(),
                addr: config.addr_user(),
                batch_config: config.batch_config(),
                max_connections: config.rpc_max_connections,
                max_payload_in_mb: config.rpc_max_request_size,
                max_payload_out_mb: config.rpc_max_response_size,
                max_subs_per_conn: config.rpc_max_subscriptions_per_connection,
                message_buffer_capacity: config.rpc_message_buffer_capacity_per_connection,
                methods: methods_user,
                metrics: metrics.clone(),
                cors: config.cors(),
                rpc_version_default: mp_chain_config::RpcVersion::RPC_VERSION_LATEST,
            })
        } else {
            None
        };

        let server_config_admin = if config.rpc_admin {
            let api_rpc_admin = rpc_api_admin(&starknet)?;
            let methods_admin = rpc_api_build("admin", api_rpc_admin).into();

            Some(ServerConfig {
                name: "JSON-RPC (Admin)".to_string(),
                addr: config.addr_admin(),
                batch_config: config.batch_config(),
                max_connections: config.rpc_max_connections,
                max_payload_in_mb: config.rpc_max_request_size,
                max_payload_out_mb: config.rpc_max_response_size,
                max_subs_per_conn: config.rpc_max_subscriptions_per_connection,
                message_buffer_capacity: config.rpc_message_buffer_capacity_per_connection,
                methods: methods_admin,
                metrics,
                cors: config.cors(),
                rpc_version_default: mp_chain_config::RpcVersion::RPC_VERSION_LATEST_ADMIN,
            })
        } else {
            None
        };

        if let Some(server_config) = &server_config_user {
            // rpc enabled
            self.server_handle_user = Some(start_server(server_config.clone(), join_set, ctx.clone()).await?);
        }

        if let Some(server_config) = &server_config_admin {
            // rpc enabled (admin)
            let ctx = ctx.child().with_id(MadaraService::RpcAdmin);
            ctx.service_add(MadaraService::RpcAdmin);
            self.server_handle_admin = Some(start_server(server_config.clone(), join_set, ctx).await?);
        }

        Ok(())
    }

    fn id(&self) -> MadaraService {
        MadaraService::Rpc
    }
}
