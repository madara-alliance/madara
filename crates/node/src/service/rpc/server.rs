#![allow(clippy::declare_interior_mutable_const)]
#![allow(clippy::borrow_interior_mutable_const)]

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Context;
use forwarded_header_value::ForwardedHeaderValue;
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, StatusCode};
use ip_network::IpNetwork;
use jsonrpsee::core::id_providers::RandomStringIdProvider;
use jsonrpsee::server::middleware::http::HostFilterLayer;
use jsonrpsee::server::middleware::rpc::RpcServiceBuilder;
use jsonrpsee::server::{stop_channel, ws, BatchRequestConfig, PingConfig, StopHandle, TowerServiceBuilder};
use jsonrpsee::{Methods, RpcModule};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tower::Service;
use tower_http::cors::{AllowOrigin, CorsLayer};

use mp_utils::wait_or_graceful_shutdown;

use super::middleware::{Metrics, MiddlewareLayer, RpcMetrics, VersionMiddlewareLayer};

const MEGABYTE: u32 = 1024 * 1024;

/// RPC server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub cors: Option<Vec<String>>,
    pub max_connections: u32,
    pub max_subs_per_conn: u32,
    pub max_payload_in_mb: u32,
    pub max_payload_out_mb: u32,
    pub metrics: RpcMetrics,
    pub message_buffer_capacity: u32,
    pub rpc_api: RpcModule<()>,
    /// Batch request config.
    pub batch_config: BatchRequestConfig,
    /// Rate limit calls per minute.
    pub rate_limit: Option<NonZeroU32>,
    /// Disable rate limit for certain ips.
    pub rate_limit_whitelisted_ips: Vec<IpNetwork>,
    /// Trust proxy headers for rate limiting.
    pub rate_limit_trust_proxy_headers: bool,
}

#[derive(Debug, Clone)]
struct PerConnection<RpcMiddleware, HttpMiddleware> {
    methods: Methods,
    stop_handle: StopHandle,
    metrics: RpcMetrics,
    service_builder: TowerServiceBuilder<RpcMiddleware, HttpMiddleware>,
}

/// Start RPC server listening on given address.
pub async fn start_server(
    config: ServerConfig,
    join_set: &mut JoinSet<anyhow::Result<()>>,
) -> anyhow::Result<jsonrpsee::server::ServerHandle> {
    let ServerConfig {
        addr,
        batch_config,
        cors,
        max_payload_in_mb,
        max_payload_out_mb,
        max_connections,
        max_subs_per_conn,
        metrics,
        message_buffer_capacity,
        rpc_api,
        rate_limit,
        rate_limit_whitelisted_ips,
        rate_limit_trust_proxy_headers,
    } = config;

    let std_listener = TcpListener::bind(addr)
        .await
        .and_then(|a| a.into_std())
        .with_context(|| format!("binding to address: {addr}"))?;
    let local_addr = std_listener.local_addr().ok();
    let host_filter = host_filtering(cors.is_some(), local_addr);

    let http_middleware = tower::ServiceBuilder::new()
		.option_layer(host_filter)
		// Proxy `GET /health` requests to internal `system_health` method.
		// .layer(ProxyGetRequestLayer::new("/health", "system_health")?)
        .layer(VersionMiddlewareLayer)
		.layer(try_into_cors(cors.as_ref())?);

    let builder = jsonrpsee::server::Server::builder()
        .max_request_body_size(max_payload_in_mb.saturating_mul(MEGABYTE))
        .max_response_body_size(max_payload_out_mb.saturating_mul(MEGABYTE))
        .max_connections(max_connections)
        .max_subscriptions_per_connection(max_subs_per_conn)
        .enable_ws_ping(
            PingConfig::new()
                .ping_interval(Duration::from_secs(30))
                .inactive_limit(Duration::from_secs(60))
                .max_failures(3),
        )
        .set_http_middleware(http_middleware)
        .set_message_buffer_capacity(message_buffer_capacity)
        .set_batch_request_config(batch_config)
        .set_id_provider(RandomStringIdProvider::new(16));

    let (stop_handle, server_handle) = stop_channel();
    let cfg = PerConnection {
        methods: build_rpc_api(rpc_api).into(),
        service_builder: builder.to_service_builder(),
        metrics,
        stop_handle: stop_handle.clone(),
    };

    let make_service = make_service_fn(move |addr: &AddrStream| {
        let cfg = cfg.clone();
        let rate_limit_whitelisted_ips = rate_limit_whitelisted_ips.clone();
        let ip = addr.remote_addr().ip();

        async move {
            let cfg = cfg.clone();
            let rate_limit_whitelisted_ips = rate_limit_whitelisted_ips.clone();

            Ok::<_, Infallible>(service_fn(move |req| {
                let proxy_ip = if rate_limit_trust_proxy_headers { get_proxy_ip(&req) } else { None };

                let rate_limit_cfg = if rate_limit_whitelisted_ips
                    .iter()
                    .any(|ips| ips.contains(proxy_ip.unwrap_or(ip)))
                {
                    log::debug!(target: "rpc", "ip={ip}, proxy_ip={:?} is trusted, disabling rate-limit", proxy_ip);
                    None
                } else {
                    if !rate_limit_whitelisted_ips.is_empty() {
                        log::debug!(target: "rpc", "ip={ip}, proxy_ip={:?} is not trusted, rate-limit enabled", proxy_ip);
                    }
                    rate_limit
                };

                let PerConnection { service_builder, metrics, stop_handle, methods } = cfg.clone();

                let is_websocket = ws::is_upgrade_request(&req);
                let transport_label = if is_websocket { "ws" } else { "http" };

                let middleware_layer = match rate_limit_cfg {
                    None => MiddlewareLayer::new().with_metrics(Metrics::new(metrics, transport_label)),
                    Some(rate_limit) => MiddlewareLayer::new()
                        .with_metrics(Metrics::new(metrics, transport_label))
                        .with_rate_limit_per_minute(rate_limit),
                };

                let rpc_middleware = RpcServiceBuilder::new().layer(middleware_layer.clone());

                let mut svc = service_builder.set_rpc_middleware(rpc_middleware).build(methods, stop_handle);

                async move {
                    if req.uri().path() == "/health" {
                        Ok(Response::builder().status(StatusCode::OK).body(Body::from("OK"))?)
                    } else {
                        if is_websocket {
                            let on_disconnect = svc.on_session_closed();

                            // Spawn a task to handle when the connection is closed.
                            tokio::spawn(async move {
                                let now = std::time::Instant::now();
                                middleware_layer.ws_connect();
                                on_disconnect.await;
                                middleware_layer.ws_disconnect(now);
                            });
                        }

                        svc.call(req).await
                    }
                }
            }))
        }
    });

    let server = hyper::Server::from_tcp(std_listener)
        .with_context(|| format!("Creating hyper server at: {addr}"))?
        .serve(make_service);

    join_set.spawn(async move {
        log::info!(
            "📱 Running JSON-RPC server at {} (allowed origins={})",
            local_addr.map_or_else(|| "unknown".to_string(), |a| a.to_string()),
            format_cors(cors.as_ref())
        );
        server
            .with_graceful_shutdown(async {
                wait_or_graceful_shutdown(stop_handle.shutdown()).await;
            })
            .await
            .context("Running rpc server")
    });

    Ok(server_handle)
}

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const X_REAL_IP: HeaderName = HeaderName::from_static("x-real-ip");
const FORWARDED: HeaderName = HeaderName::from_static("forwarded");

pub(crate) fn host_filtering(enabled: bool, addr: Option<SocketAddr>) -> Option<HostFilterLayer> {
    // If the local_addr failed, fallback to wildcard.
    let port = addr.map_or("*".to_string(), |p| p.port().to_string());

    if enabled {
        // NOTE: The listening addresses are whitelisted by default.
        let hosts = [format!("localhost:{port}"), format!("127.0.0.1:{port}"), format!("[::1]:{port}")];
        Some(HostFilterLayer::new(hosts).expect("Invalid host filter"))
    } else {
        None
    }
}

pub(crate) fn build_rpc_api<M: Send + Sync + 'static>(mut rpc_api: RpcModule<M>) -> RpcModule<M> {
    let mut available_methods = rpc_api.method_names().collect::<Vec<_>>();

    // The "rpc_methods" is defined below and we want it to be part of the reported methods.
    // The available methods will be prefixed by their version, example:
    // * starknet_V0_7_1_blockNumber,
    // * starknet_V0_8_0_blockNumber (...)
    available_methods.push("rpc_methods");
    available_methods.sort();

    rpc_api
        .register_method("rpc_methods", move |_, _| {
            serde_json::json!({
                "methods": available_methods,
            })
        })
        .expect("Cannot register method");

    rpc_api
}

pub(crate) fn try_into_cors(maybe_cors: Option<&Vec<String>>) -> anyhow::Result<CorsLayer> {
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

pub(crate) fn format_cors(maybe_cors: Option<&Vec<String>>) -> String {
    if let Some(cors) = maybe_cors {
        format!("{:?}", cors)
    } else {
        format!("{:?}", ["*"])
    }
}

/// Extracts the IP addr from the HTTP request.
///
/// It is extracted in the following order:
/// 1. `Forwarded` header.
/// 2. `X-Forwarded-For` header.
/// 3. `X-Real-Ip`.
pub(crate) fn get_proxy_ip(req: &Request<hyper::Body>) -> Option<IpAddr> {
    if let Some(ip) = req
        .headers()
        .get(&FORWARDED)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| ForwardedHeaderValue::from_forwarded(v).ok())
        .and_then(|v| v.remotest_forwarded_for_ip())
    {
        return Some(ip);
    }

    if let Some(ip) = req
        .headers()
        .get(&X_FORWARDED_FOR)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| ForwardedHeaderValue::from_x_forwarded_for(v).ok())
        .and_then(|v| v.remotest_forwarded_for_ip())
    {
        return Some(ip);
    }

    if let Some(ip) = req.headers().get(&X_REAL_IP).and_then(|v| v.to_str().ok()).and_then(|v| IpAddr::from_str(v).ok())
    {
        return Some(ip);
    }

    None
}
