#![allow(clippy::declare_interior_mutable_const)]
#![allow(clippy::borrow_interior_mutable_const)]

use super::metrics::RpcMetrics;
use super::middleware::{Metrics, RpcMiddlewareLayerMetrics};
use crate::service::rpc::middleware::RpcMiddlewareServiceVersion;
use anyhow::Context;
use jsonrpsee::server::middleware::rpc::RpcServiceBuilder;
use mc_rpc::versions::user::v0_7_1::methods::read::syncing::syncing;
use mc_rpc::Starknet;
use mp_chain_config::RpcVersion;
use mp_rpc::v0_7_1::SyncingStatus;
use mp_utils::service::ServiceContext;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower::Service;

#[allow(non_upper_case_globals)]
const MiB: u32 = 1024 * 1024;

/// RPC server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub addr: SocketAddr,
    pub cors: Option<Vec<String>>,
    pub rpc_version_default: mp_chain_config::RpcVersion,
    pub max_connections: u32,
    pub max_subs_per_conn: u32,
    pub max_payload_in_mib: u32,
    pub max_payload_out_mib: u32,
    pub metrics: RpcMetrics,
    pub message_buffer_capacity: u32,
    pub methods: jsonrpsee::Methods,
    /// Batch request config.
    pub batch_config: jsonrpsee::server::BatchRequestConfig,
    pub supported_versions: Vec<RpcVersion>,
}

#[derive(Debug, Clone)]
struct PerConnection<RpcMiddleware, HttpMiddleware> {
    methods: jsonrpsee::Methods,
    stop_handle: jsonrpsee::server::StopHandle,
    metrics: RpcMetrics,
    service_builder: jsonrpsee::server::TowerServiceBuilder<RpcMiddleware, HttpMiddleware>,
}

/// Start RPC server listening on given address.
///
/// This future will complete once the server has been shutdown.
pub async fn start_server(
    config: ServerConfig,
    mut ctx: ServiceContext,
    stop_handle: jsonrpsee::server::StopHandle,
    starknet: Arc<Starknet>,
) -> anyhow::Result<()> {
    let ServerConfig {
        name,
        addr,
        cors,
        rpc_version_default,
        max_connections,
        max_subs_per_conn,
        max_payload_in_mib,
        max_payload_out_mib,
        metrics,
        message_buffer_capacity,
        methods,
        batch_config,
        supported_versions,
    } = config;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Binding TCP listener to address: {addr}"))?;
    let local_addr = listener.local_addr().context("Failed to retrieve local address after binding TCP listener")?;

    let ping_config = jsonrpsee::server::PingConfig::new()
        .ping_interval(Duration::from_secs(30))
        .inactive_limit(Duration::from_secs(60))
        .max_failures(3);

    let http_middleware = tower::ServiceBuilder::new()
        .option_layer(host_filtering(cors.is_some(), local_addr))
        .layer(try_into_cors(cors.as_ref())?);

    let server_config = jsonrpsee::server::ServerConfig::builder()
        .max_request_body_size(max_payload_in_mib.saturating_mul(MiB))
        .max_response_body_size(max_payload_out_mib.saturating_mul(MiB))
        .max_connections(max_connections)
        .max_subscriptions_per_connection(max_subs_per_conn)
        .enable_ws_ping(ping_config)
        .set_message_buffer_capacity(message_buffer_capacity)
        .set_batch_request_config(batch_config)
        .set_id_provider(jsonrpsee::server::RandomStringIdProvider::new(16))
        .build();

    let builder = jsonrpsee::server::Server::builder().set_config(server_config).set_http_middleware(http_middleware);

    let cfg = PerConnection {
        methods,
        stop_handle: stop_handle.clone(),
        metrics,
        service_builder: builder.to_service_builder(),
    };
    let mut connections = tokio::task::JoinSet::new();

    tracing::info!(
        "📱 Running {name} server at http://{}/rpc/v{}/ (allowed origins={}, supported versions={})",
        local_addr.to_string(),
        config.rpc_version_default,
        format_cors(cors.as_ref()),
        format_rpc_versions(&supported_versions),
    );

    while let Some(res) = ctx.run_until_cancelled(listener.accept()).await {
        let (stream, _) = res.context("Accepting RPC connection")?;
        let cfg = cfg.clone();
        let stop_handle_for_service = cfg.stop_handle.clone();
        let ctx_for_service = ctx.clone();
        let mut ctx_for_shutdown = ctx.clone();
        let starknet = Arc::clone(&starknet);

        connections.spawn(async move {
            let service = tower::service_fn(move |req| {
                let PerConnection { service_builder, metrics, stop_handle, methods } = cfg.clone();
                let ctx1 = ctx_for_service.clone();
                let starknet = Arc::clone(&starknet);

                let is_websocket = jsonrpsee::server::ws::is_upgrade_request(&req);
                let transport_label = if is_websocket { "ws" } else { "http" };
                let path = req.uri().path().to_string();
                let metrics_layer = RpcMiddlewareLayerMetrics::new(Metrics::new(metrics, transport_label));

                let rpc_middleware = RpcServiceBuilder::new()
                    .layer_fn(move |service| {
                        RpcMiddlewareServiceVersion::new(service, path.clone(), rpc_version_default)
                    })
                    .layer(metrics_layer.clone());

                let mut svc = service_builder.set_rpc_middleware(rpc_middleware).build(methods, stop_handle);

                async move {
                    if ctx1.is_cancelled() {
                        Ok::<_, Infallible>(text_response(http::StatusCode::GONE, "GONE"))
                    } else if req.uri().path() == "/health" {
                        Ok::<_, Infallible>(text_response(http::StatusCode::OK, "OK"))
                    } else if req.uri().path() == "/ready" {
                        let sync_status = syncing(&starknet);
                        match sync_status {
                            Ok(sync_status) => match sync_status {
                                SyncingStatus::Syncing(_) => {
                                    Ok(text_response(http::StatusCode::SERVICE_UNAVAILABLE, "SYNCING"))
                                }
                                SyncingStatus::NotSyncing => Ok(text_response(http::StatusCode::OK, "OK")),
                            },
                            Err(_) => {
                                Ok(text_response(http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_SERVER_ERROR"))
                            }
                        }
                    } else {
                        if is_websocket {
                            // Utilize the session close future to know when the actual WebSocket
                            // session was closed.
                            let on_disconnect = svc.on_session_closed();

                            // Spawn a task to handle when the connection is closed.
                            tokio::spawn(async move {
                                let now = std::time::Instant::now();
                                metrics_layer.ws_connect();
                                on_disconnect.await;
                                metrics_layer.ws_disconnect(now);
                            });
                        }

                        match svc.call(req).await {
                            Ok(response) => Ok(response),
                            Err(error) => {
                                tracing::error!("Error handling RPC request: {error:#}");
                                Ok(text_response(http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_SERVER_ERROR"))
                            }
                        }
                    }
                }
            });

            let stopped = async move {
                ctx_for_shutdown.run_until_cancelled(stop_handle_for_service.shutdown()).await;
            };

            if let Err(err) = jsonrpsee::server::serve_with_graceful_shutdown(stream, service, stopped).await {
                tracing::error!("Error serving RPC connection: {err:#}");
            }
        });
    }

    while let Some(res) = connections.join_next().await {
        if let Err(error) = res {
            tracing::error!("RPC connection task failed: {error:#}");
        }
    }

    Ok(())
}

fn text_response(status: http::StatusCode, body: &'static str) -> jsonrpsee::server::HttpResponse {
    let mut response = http::Response::new(jsonrpsee::server::HttpBody::from(body));
    *response.status_mut() = status;
    response
}

// Copied from https://github.com/paritytech/polkadot-sdk/blob/a0aefc6b233ace0a82a8631d67b6854e6aeb014b/substrate/client/rpc-servers/src/utils.rs#L192
pub(crate) fn host_filtering(
    enabled: bool,
    addr: SocketAddr,
) -> Option<jsonrpsee::server::middleware::http::HostFilterLayer> {
    if enabled {
        // NOTE: The listening addresses are whitelisted by default.

        let mut hosts = Vec::new();

        if addr.is_ipv4() {
            hosts.push(format!("localhost:{}", addr.port()));
            hosts.push(format!("127.0.0.1:{}", addr.port()));
        } else {
            hosts.push(format!("[::1]:{}", addr.port()));
        }

        Some(jsonrpsee::server::middleware::http::HostFilterLayer::new(hosts).expect("Valid hosts; qed"))
    } else {
        None
    }
}

pub(crate) fn rpc_api_build<M: Send + Sync + 'static>(
    service: &str,
    mut rpc_api: jsonrpsee::RpcModule<M>,
) -> jsonrpsee::RpcModule<M> {
    let mut available_methods = rpc_api
        .method_names()
        .map(|name| {
            let split = name.split("_").collect::<Vec<_>>();

            if split.len() == 2 {
                // method is version-agnostic
                let namespace = split[0];
                let method = split[1];
                format!("{service}/{namespace}_{method}")
            } else {
                // versioned method
                let namespace = split[0];
                let major = split[1];
                let minor = split[2];
                let patch = split[3];
                let method = split[4];
                format!("{service}/{major}_{minor}_{patch}/{namespace}_{method}")
            }
        })
        .collect::<Vec<_>>();

    // The "rpc_methods" is defined below and we want it to be part of the reported methods.
    // The available methods will be prefixed by their version, example:
    // * rpc/v0_7_1/starknet_blockNumber,
    // * rpc/v0_8_1/starknet_blockNumber (...)
    available_methods.push(format!("{service}/rpc_methods"));
    available_methods.sort();

    rpc_api
        .register_method("rpc_methods", move |_, _, _| {
            serde_json::json!({
                "methods": available_methods,
            })
        })
        .expect("Cannot register method");

    rpc_api
}

pub(crate) fn try_into_cors(maybe_cors: Option<&Vec<String>>) -> anyhow::Result<tower_http::cors::CorsLayer> {
    if let Some(cors) = maybe_cors {
        let mut list = Vec::new();
        for origin in cors {
            list.push(http::header::HeaderValue::from_str(origin)?);
        }
        Ok(tower_http::cors::CorsLayer::new().allow_origin(tower_http::cors::AllowOrigin::list(list)))
    } else {
        Ok(tower_http::cors::CorsLayer::permissive())
    }
}

pub(crate) fn format_cors(maybe_cors: Option<&Vec<String>>) -> String {
    if let Some(cors) = maybe_cors {
        format!("{:?}", cors)
    } else {
        format!("{:?}", ["*"])
    }
}
pub(crate) fn format_rpc_versions(versions: &[RpcVersion]) -> String {
    format!("{:?}", versions.iter().map(|v| format!("/rpc/v{v}/")).collect::<Vec<_>>())
}
