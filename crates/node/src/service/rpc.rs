use std::sync::Arc;

use anyhow::bail;
use jsonrpsee::server::ServerHandle;
use jsonrpsee::RpcModule;
use tokio::task::JoinSet;

use dc_db::DatabaseService;
use dc_metrics::MetricsRegistry;
use dc_rpc::{providers::AddTransactionProvider, versions::v0_7_1, Starknet};
use dp_chain_config::{ChainConfig, RpcVersion};
use dp_utils::service::Service;

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
        chain_config: Arc<ChainConfig>,
        metrics_handle: MetricsRegistry,
        add_txs_method_provider: Arc<dyn AddTransactionProvider>,
    ) -> anyhow::Result<Self> {
        if config.rpc_disabled {
            return Ok(Self { server_config: None, server_handle: None });
        }

        let (rpcs, _node_operator) = match (config.rpc_methods, config.rpc_external) {
            (RpcMethods::Safe, _) => (true, false),
            (RpcMethods::Unsafe, _) => (true, true),
            (RpcMethods::Auto, false) => (true, true),
            (RpcMethods::Auto, true) => {
                log::warn!(
                    "Option `--rpc-external` will hide node operator endpoints. To enable them, please pass \
                     `--rpc-methods unsafe`."
                );
                (true, false)
            }
        };
        let (read, write, trace) = (rpcs, rpcs, rpcs);

        let starknet = Starknet::new(Arc::clone(db.backend()), chain_config.clone(), add_txs_method_provider);

        let mut rpc_api = RpcModule::new(());
        configure_with_supported_apis(&mut rpc_api, &starknet, read, write, trace)?;

        let metrics = RpcMetrics::register(&metrics_handle)?;

        Ok(Self {
            server_config: Some(ServerConfig {
                addr: config.addr(),
                batch_config: config.batch_config(),
                max_connections: config.rpc_max_connections,
                max_payload_in_mb: config.rpc_max_request_size,
                max_payload_out_mb: config.rpc_max_response_size,
                max_subs_per_conn: config.rpc_max_subscriptions_per_connection,
                message_buffer_capacity: config.rpc_message_buffer_capacity_per_connection,
                rpc_api,
                metrics,
                cors: config.cors(),
                rate_limit: config.rpc_rate_limit,
                rate_limit_whitelisted_ips: config.rpc_rate_limit_whitelisted_ips.clone(),
                rate_limit_trust_proxy_headers: config.rpc_rate_limit_trust_proxy_headers,
            }),
            server_handle: None,
        })
    }
}

fn configure_with_supported_apis(
    rpc_api: &mut jsonrpsee::RpcModule<()>,
    starknet: &Starknet,
    read: bool,
    write: bool,
    trace: bool,
) -> anyhow::Result<()> {
    for rpc_version in dp_chain_config::SUPPORTED_RPC_VERSIONS.iter() {
        match *rpc_version {
            RpcVersion::RPC_VERSION_0_6_0 => todo!(),
            RpcVersion::RPC_VERSION_0_7_0 => todo!(),
            RpcVersion::RPC_VERSION_0_7_1 => {
                if read {
                    rpc_api.merge(v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
                if write {
                    rpc_api.merge(v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
                if trace {
                    rpc_api.merge(v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
            }
            _ => bail!("Unrecognized RPC spec version: {}", rpc_version),
        }
    }
    Ok(())
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
