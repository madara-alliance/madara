use std::sync::Arc;

use jsonrpsee::server::ServerHandle;
use tokio::task::JoinSet;

use mc_db::DatabaseService;
use mc_rpc::{providers::AddTransactionProvider, versioned_rpc_api, Starknet};
use mp_utils::service::Service;

use metrics::RpcMetrics;
use server::{start_server, ServerConfig};

use crate::cli::{RpcMethods, RpcParams};

mod metrics;
mod middleware;
mod server;

pub struct RpcService {
    server_config: Option<ServerConfig>,
    server_handle: Option<ServerHandle>,
}
impl RpcService {
    pub fn new(
        config: &RpcParams,
        db: &DatabaseService,
        add_txs_method_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        if config.rpc_disabled {
            return Ok(Self { server_config: None, server_handle: None });
        }

        let (rpcs, node_operator) = match (config.rpc_methods, config.rpc_external) {
            (RpcMethods::Safe, _) => (true, false),
            (RpcMethods::Unsafe, _) => (true, true),
            (RpcMethods::Auto, false) => (true, true),
            (RpcMethods::Auto, true) => {
                tracing::warn!(
                    "Option `--rpc-external` will hide node operator endpoints. To enable them, please pass \
                     `--rpc-methods unsafe`."
                );
                (true, false)
            }
        };
        let (read, write, trace, internal, ws) = (rpcs, rpcs, rpcs, node_operator, rpcs);
        let starknet = Starknet::new(Arc::clone(db.backend()), add_txs_method_provider);
        let metrics = RpcMetrics::register()?;

        Ok(Self {
            server_config: Some(ServerConfig {
                addr: config.addr(),
                batch_config: config.batch_config(),
                max_connections: config.rpc_max_connections,
                max_payload_in_mb: config.rpc_max_request_size,
                max_payload_out_mb: config.rpc_max_response_size,
                max_subs_per_conn: config.rpc_max_subscriptions_per_connection,
                message_buffer_capacity: config.rpc_message_buffer_capacity_per_connection,
                rpc_api: versioned_rpc_api(&starknet, read, write, trace, internal, ws)?,
                metrics,
                cors: config.cors(),
                rate_limit: config.rpc_rate_limit,
            }),
            server_handle: None,
        })
    }
}

#[async_trait::async_trait]
impl Service for RpcService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if let Some(server_config) = &self.server_config {
            // rpc enabled
            self.server_handle = Some(start_server(server_config.clone(), join_set).await?);
        }

        Ok(())
    }
}
