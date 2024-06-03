mod database;
use std::os::unix::net::SocketAddr;

pub use database::*;
use jsonrpsee::server::middleware::proxy_get_request::ProxyGetRequestLayer;
use tower::ServiceBuilder;

use crate::cli::{RpcMethods, RpcParams};

pub struct RpcService {}

const MEGABYTE: u32 = 1024 * 1024;

impl RpcService {
    pub fn new(config: RpcParams) -> CliResult<Self> {
        let middleware = ServiceBuilder::new()
		// Proxy `GET /health` requests to internal `system_health` method.
		.layer(ProxyGetRequestLayer::new("/health", "system_health")?)
		.layer(try_into_cors(cors)?);

        let mut builder = ServerBuilder::new()
            .max_request_body_size(max_payload_in_mb.saturating_mul(MEGABYTE))
            .max_response_body_size(max_payload_out_mb.saturating_mul(MEGABYTE))
            .max_connections(max_connections)
            .max_subscriptions_per_connection(max_subs_per_conn)
            .ping_interval(std::time::Duration::from_secs(30))
            .set_host_filtering(host_filter)
            .set_middleware(middleware)
            .custom_tokio_runtime(tokio_handle);

        if let Some(provider) = id_provider {
            builder = builder.set_id_provider(provider);
        } else {
            builder = builder.set_id_provider(RandomStringIdProvider::new(16));
        };

        let rpc_api = build_rpc_api(rpc_api);
        let (handle, addr) = if let Some(metrics) = metrics {
            let server = builder.set_logger(metrics).build(&addrs[..]).await?;
            let addr = server.local_addr();
            (server.start(rpc_api)?, addr)
        } else {
            let server = builder.build(&addrs[..]).await?;
            let addr = server.local_addr();
            (server.start(rpc_api)?, addr)
        };

        log::info!(
            "Running JSON-RPC server: addr={}, allowed origins={}",
            addr.map_or_else(|_| "unknown".to_string(), |a| a.to_string()),
            format_cors(cors)
        );

        todo!()
    }
}

fn hosts_filtering(enabled: bool, addrs: &[SocketAddr]) -> AllowHosts {
    if enabled {
        // NOTE The listening addresses are whitelisted by default.
        let mut hosts = Vec::with_capacity(addrs.len() * 2);
        for addr in addrs {
            hosts.push(format!("localhost:{}", addr.port()).into());
            hosts.push(format!("127.0.0.1:{}", addr.port()).into());
        }
        AllowHosts::Only(hosts)
    } else {
        AllowHosts::Any
    }
}

fn build_rpc_api<M: Send + Sync + 'static>(mut rpc_api: RpcModule<M>) -> RpcModule<M> {
    let mut available_methods = rpc_api.method_names().collect::<Vec<_>>();
    // The "rpc_methods" is defined below and we want it to be part of the reported methods.
    available_methods.push("rpc_methods");
    available_methods.sort();

    rpc_api
        .register_method("rpc_methods", move |_, _| {
            Ok(serde_json::json!({
                "methods": available_methods,
            }))
        })
        .expect("infallible all other methods have their own address space; qed");

    rpc_api
}

fn try_into_cors(maybe_cors: Option<&Vec<String>>) -> Result<CorsLayer, Box<dyn StdError + Send + Sync>> {
    if let Some(cors) = maybe_cors {
        let mut list = Vec::new();
        for origin in cors {
            list.push(HeaderValue::from_str(origin)?);
        }
        Ok(CorsLayer::new().allow_origin(AllowOrigin::list(list)))
    } else {
        // allow all cors
        Ok(CorsLayer::permissive())
    }
}

fn format_cors(maybe_cors: Option<&Vec<String>>) -> String {
    if let Some(cors) = maybe_cors { format!("{:?}", cors) } else { format!("{:?}", ["*"]) }
}
