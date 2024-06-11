use jsonrpsee::server::ServerHandle;
use jsonrpsee::RpcModule;
use mc_metrics::MetricsRegistry;
use mc_rpc::{
    ChainConfig, Felt, Starknet, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer,
};
use metrics::RpcMetrics;
use server::{start_server, ServerConfig};
use tokio::task::JoinSet;

use crate::cli::{NetworkType, RpcMethods, RpcParams};

mod metrics;
mod middleware;
mod server;

pub struct RpcService {
    server_config: ServerConfig,
    server_handle: Option<ServerHandle>,
}
impl RpcService {
    pub fn new(config: &RpcParams, network_type: NetworkType, metrics_handle: MetricsRegistry) -> anyhow::Result<Self> {
        let mut rpc_api = RpcModule::new(());

        let (read, write, trace) = match (config.rpc_methods, config.rpc_external) {
            (RpcMethods::Safe, _) => (true, false, false),
            (RpcMethods::Unsafe, _) => (true, true, true),
            (RpcMethods::Auto, false) => (true, true, true),
            (RpcMethods::Auto, true) => {
                log::warn!(
                    "Option `--rpc-external` will hide Write and Trace endpoints. To enable them, please pass \
                     `--rpc-methods unsafe`."
                );
                (true, false, false)
            }
        };

        let chain_config = ChainConfig {
            chain_id: Felt(network_type.chain_id()),
            feeder_gateway: network_type.feeder_gateway(),
            gateway: network_type.gateway(),
        };

        if read {
            // TODO: staring block
            rpc_api.merge(StarknetReadRpcApiServer::into_rpc(Starknet::new(0, chain_config.clone())))?;
        }
        if write {
            rpc_api.merge(StarknetWriteRpcApiServer::into_rpc(Starknet::new(0, chain_config.clone())))?;
        }
        if trace {
            rpc_api.merge(StarknetTraceRpcApiServer::into_rpc(Starknet::new(0, chain_config.clone())))?;
        }

        let metrics = RpcMetrics::register(&metrics_handle)?;

        Ok(Self {
            server_config: ServerConfig {
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
            },
            server_handle: None,
        })
    }
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        self.server_handle = Some(start_server(self.server_config.clone(), join_set).await?);
        Ok(())
    }
}
