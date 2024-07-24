use dc_db::DatabaseService;
use dc_metrics::MetricsRegistry;
use dc_rpc::{
    providers::ForwardToProvider, ChainConfig, Starknet, StarknetReadRpcApiServer, StarknetTraceRpcApiServer,
    StarknetWriteRpcApiServer,
};
use jsonrpsee::server::ServerHandle;
use jsonrpsee::RpcModule;
use metrics::RpcMetrics;
use server::{start_server, ServerConfig};
use starknet_providers::SequencerGatewayProvider;
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tokio::task::JoinSet;

use crate::cli::{NetworkType, RpcMethods, RpcParams};

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
        network_type: NetworkType,
        metrics_handle: MetricsRegistry,
    ) -> anyhow::Result<Self> {
        if config.rpc_disabled {
            return Ok(Self { server_config: None, server_handle: None });
        }

        let mut rpc_api = RpcModule::new(());

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

        let chain_config = ChainConfig {
            chain_id: Felt::from_bytes_be_slice(network_type.chain_id().as_bytes()),
            feeder_gateway: network_type.feeder_gateway(),
            gateway: network_type.gateway(),
        };

        let starknet = Starknet::new(
            Arc::clone(db.backend()),
            chain_config.clone(),
            // TODO(rate-limit): we may get rate limited with this unconfigured provider?
            Arc::new(ForwardToProvider::new(SequencerGatewayProvider::new(
                chain_config.gateway.clone(),
                chain_config.feeder_gateway.clone(),
                chain_config.chain_id,
            ))),
        );

        if read {
            rpc_api.merge(StarknetReadRpcApiServer::into_rpc(starknet.clone()))?;
        }
        if write {
            rpc_api.merge(StarknetWriteRpcApiServer::into_rpc(starknet.clone()))?;
        }
        if trace {
            rpc_api.merge(StarknetTraceRpcApiServer::into_rpc(starknet.clone()))?;
        }

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
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if let Some(server_config) = &self.server_config {
            // rpc enabled
            self.server_handle = Some(start_server(server_config.clone(), join_set).await?);
        }

        Ok(())
    }
}
