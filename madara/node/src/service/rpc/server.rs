#![allow(clippy::declare_interior_mutable_const)]
#![allow(clippy::borrow_interior_mutable_const)]

use super::metrics::RpcMetrics;
use super::middleware::{Metrics, RpcExecutionLimiterLayer, RpcMiddlewareLayerMetrics};
use crate::service::rpc::middleware::RpcMiddlewareServiceVersion;
use anyhow::Context;
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
const WS_PING_INTERVAL_SECS: u64 = 30;
const WS_MAX_PING_FAILURES: usize = 3;

/// RPC server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub addr: SocketAddr,
    pub cors: Option<Vec<String>>,
    pub rpc_version_default: mp_chain_config::RpcVersion,
    pub max_connections: u32,
    pub max_concurrent_executions: usize,
    pub max_subs_per_conn: u32,
    pub ws_inactive_timeout_secs: u64,
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
    ctx: ServiceContext,
    stop_handle: jsonrpsee::server::StopHandle,
    starknet: Arc<Starknet>,
) -> anyhow::Result<()> {
    let addr = config.addr;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Binding TCP listener to address: {addr}"))?;

    serve(listener, config, ctx, stop_handle, starknet).await
}

async fn serve(
    listener: tokio::net::TcpListener,
    config: ServerConfig,
    mut ctx: ServiceContext,
    stop_handle: jsonrpsee::server::StopHandle,
    starknet: Arc<Starknet>,
) -> anyhow::Result<()> {
    let ServerConfig {
        name,
        addr: _,
        cors,
        rpc_version_default,
        max_connections,
        max_concurrent_executions,
        max_subs_per_conn,
        ws_inactive_timeout_secs,
        max_payload_in_mib,
        max_payload_out_mib,
        metrics,
        message_buffer_capacity,
        methods,
        batch_config,
        supported_versions,
    } = config;

    let local_addr = listener.local_addr().context("Failed to retrieve local address after binding TCP listener")?;

    let ping_config = jsonrpsee::server::PingConfig::new()
        .ping_interval(Duration::from_secs(WS_PING_INTERVAL_SECS))
        .inactive_limit(Duration::from_secs(ws_inactive_timeout_secs))
        .max_failures(WS_MAX_PING_FAILURES);

    let http_middleware = tower::ServiceBuilder::new()
        .option_layer(host_filtering(cors.is_some(), local_addr))
        .layer(try_into_cors(cors.as_ref())?);

    let builder = jsonrpsee::server::Server::builder()
        .max_request_body_size(max_payload_in_mib.saturating_mul(MiB))
        .max_response_body_size(max_payload_out_mib.saturating_mul(MiB))
        .max_connections(max_connections)
        .max_subscriptions_per_connection(max_subs_per_conn)
        .set_id_provider(mc_rpc::StarknetSubscriptionIdProvider::default())
        .enable_ws_ping(ping_config)
        .set_message_buffer_capacity(message_buffer_capacity)
        .set_batch_request_config(batch_config)
        .set_http_middleware(http_middleware);

    let cfg = PerConnection {
        methods,
        stop_handle: stop_handle.clone(),
        metrics,
        service_builder: builder.to_service_builder(),
    };
    let execution_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_executions));
    let ctx1 = ctx.clone();

    let make_service = hyper::service::make_service_fn(move |_| {
        let cfg = cfg.clone();
        let ctx1 = ctx1.clone();
        let starknet = Arc::clone(&starknet);
        let execution_semaphore = Arc::clone(&execution_semaphore);

        async move {
            let cfg = cfg.clone();
            let starknet = Arc::clone(&starknet);
            let execution_semaphore = Arc::clone(&execution_semaphore);

            Ok::<_, Infallible>(hyper::service::service_fn(move |req| {
                let PerConnection { service_builder, metrics, stop_handle, methods } = cfg.clone();
                let ctx1 = ctx1.clone();
                let starknet = Arc::clone(&starknet);
                let execution_semaphore = Arc::clone(&execution_semaphore);

                let is_websocket = jsonrpsee::server::ws::is_upgrade_request(&req);
                let transport_label = if is_websocket { "ws" } else { "http" };
                let path = req.uri().path().to_string();
                let metrics_layer = RpcMiddlewareLayerMetrics::new(Metrics::new(metrics, transport_label));

                let rpc_middleware = jsonrpsee::server::RpcServiceBuilder::new()
                    .layer_fn(move |service| {
                        RpcMiddlewareServiceVersion::new(service, path.clone(), rpc_version_default)
                    })
                    .layer(metrics_layer.clone())
                    .layer(RpcExecutionLimiterLayer::new(execution_semaphore));

                let mut svc = service_builder.set_rpc_middleware(rpc_middleware).build(methods, stop_handle);

                async move {
                    if ctx1.is_cancelled() {
                        Ok(hyper::Response::builder()
                            .status(hyper::StatusCode::GONE)
                            .body(hyper::Body::from("GONE"))?)
                    } else if req.uri().path() == "/health" {
                        Ok(hyper::Response::builder().status(hyper::StatusCode::OK).body(hyper::Body::from("OK"))?)
                    } else if req.uri().path() == "/ready" {
                        let sync_status = syncing(&starknet);
                        match sync_status {
                            Ok(sync_status) => match sync_status {
                                SyncingStatus::Syncing(_) => Ok(hyper::Response::builder()
                                    .status(hyper::StatusCode::SERVICE_UNAVAILABLE)
                                    .body(hyper::Body::from("SYNCING"))?),
                                SyncingStatus::NotSyncing => Ok(hyper::Response::builder()
                                    .status(hyper::StatusCode::OK)
                                    .body(hyper::Body::from("OK"))?),
                            },
                            Err(_) => Ok(hyper::Response::builder()
                                .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
                                .body(hyper::Body::from("INTERNAL_SERVER_ERROR"))?),
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

                        svc.call(req).await
                    }
                }
            }))
        }
    });

    let server = hyper::Server::from_tcp(listener.into_std()?)
        .with_context(|| format!("Creating hyper server at: {local_addr}"))?
        .serve(make_service);

    tracing::info!(
        "Running {name} HTTP server at http://{}/rpc/v{}/ (max_connections={}, max_request_size_mib={}, max_response_size_mib={}, allowed_origins={}, supported_versions={})",
        local_addr,
        rpc_version_default,
        max_connections,
        max_payload_in_mib,
        max_payload_out_mib,
        format_cors(cors.as_ref()),
        format_rpc_versions(&supported_versions),
    );
    tracing::info!(
        "Running {name} WebSocket server at ws://{}/rpc/v{}/ (max_connections={}, max_subscriptions_per_connection={}, message_buffer_capacity_per_connection={}, ws_ping_interval_secs={}, ws_inactive_timeout_secs={}, ws_max_ping_failures={}, allowed_origins={}, supported_versions={})",
        local_addr,
        rpc_version_default,
        max_connections,
        max_subs_per_conn,
        message_buffer_capacity,
        WS_PING_INTERVAL_SECS,
        ws_inactive_timeout_secs,
        WS_MAX_PING_FAILURES,
        format_cors(cors.as_ref()),
        format_rpc_versions(&supported_versions),
    );

    server
        .with_graceful_shutdown(async {
            ctx.run_until_cancelled(stop_handle.shutdown()).await;
        })
        .await
        .context("Running rpc server")
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
        .register_method("rpc_methods", move |_, _| {
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
            list.push(hyper::header::HeaderValue::from_str(origin)?);
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

#[cfg(test)]
mod tests {
    use super::{serve, ServerConfig};
    use async_trait::async_trait;
    use jsonrpsee::{
        core::client::ClientT, rpc_params, server::BatchRequestConfig, ws_client::WsClientBuilder, RpcModule,
    };
    use mc_db::MadaraBackend;
    use mc_rpc::Starknet;
    use mc_submit_tx::{SubmitTransaction, SubmitTransactionError, TransactionLookup};
    use mp_chain_config::{ChainConfig, RpcVersion};
    use mp_rpc::{
        admin::BroadcastedDeclareTxnV0,
        v0_10_2::BroadcastedInvokeTxn,
        v0_9_0::{
            AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, ClassAndTxnHash,
            ContractAndTxnHash,
        },
    };
    use mp_utils::service::ServiceContext;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::{mpsc, Semaphore};

    struct UnusedTransactionProvider;

    #[async_trait]
    impl SubmitTransaction for UnusedTransactionProvider {
        async fn submit_declare_transaction(
            &self,
            _tx: BroadcastedDeclareTxn,
        ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_deploy_account_transaction(
            &self,
            _tx: BroadcastedDeployAccountTxn,
        ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_invoke_transaction(
            &self,
            _tx: BroadcastedInvokeTxn,
        ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_declare_v0_transaction(
            &self,
            _tx: BroadcastedDeclareTxnV0,
        ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
            Err(SubmitTransactionError::Unsupported)
        }
    }

    #[async_trait]
    impl TransactionLookup for UnusedTransactionProvider {
        async fn received_transaction(&self, _hash: mp_convert::Felt) -> Option<bool> {
            None
        }

        async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<mp_convert::Felt>> {
            None
        }
    }

    #[derive(Clone)]
    struct BlockingRpcContext {
        started: mpsc::UnboundedSender<&'static str>,
        release: Arc<Semaphore>,
    }

    async fn blocking_response(context: Arc<BlockingRpcContext>, method: &'static str) -> &'static str {
        context.started.send(method).expect("test receiver should remain open");
        context.release.clone().acquire_owned().await.expect("test semaphore should remain open").forget();
        "ok"
    }

    fn test_starknet(ctx: ServiceContext) -> Arc<Starknet> {
        let provider = Arc::new(UnusedTransactionProvider);
        Arc::new(Starknet::new(
            MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test())),
            provider.clone(),
            provider,
            Default::default(),
            None,
            ctx,
        ))
    }

    #[tokio::test]
    async fn execution_limit_is_shared_across_connections() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        let mut module = RpcModule::new(BlockingRpcContext { started: started_tx, release: Arc::clone(&release) });
        module
            .register_async_method("starknet_V0_10_2_call", |_, context| async move {
                blocking_response(context, "starknet_call").await
            })
            .unwrap();
        module
            .register_async_method("starknet_V0_10_2_estimateFee", |_, context| async move {
                blocking_response(context, "starknet_estimateFee").await
            })
            .unwrap();
        module.register_method("starknet_V0_10_2_blockNumber", |_, _| 1_u64).unwrap();

        let ctx = ServiceContext::default();
        let (stop_handle, _server_handle) = jsonrpsee::server::stop_channel();
        let server = tokio::spawn(serve(
            listener,
            ServerConfig {
                name: "test RPC".into(),
                addr,
                cors: None,
                rpc_version_default: RpcVersion::RPC_VERSION_0_10_2,
                max_connections: 4,
                max_concurrent_executions: 1,
                max_subs_per_conn: 4,
                ws_inactive_timeout_secs: 60,
                max_payload_in_mib: 1,
                max_payload_out_mib: 1,
                metrics: super::RpcMetrics::register().unwrap(),
                message_buffer_capacity: 16,
                methods: module.into(),
                batch_config: BatchRequestConfig::Disabled,
                supported_versions: vec![RpcVersion::RPC_VERSION_0_10_2],
            },
            ctx.clone(),
            stop_handle,
            test_starknet(ctx.clone()),
        ));

        let url = format!("ws://{addr}");
        let connection_a = Arc::new(WsClientBuilder::default().build(&url).await.unwrap());
        let connection_b = Arc::new(WsClientBuilder::default().build(&url).await.unwrap());

        let first_client = Arc::clone(&connection_a);
        let first =
            tokio::spawn(
                async move { first_client.request::<String, _>("starknet_call", rpc_params![]).await.unwrap() },
            );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
                .await
                .expect("first connection should enter its handler"),
            Some("starknet_call")
        );

        let second_client = Arc::clone(&connection_b);
        let second = tokio::spawn(async move {
            second_client.request::<String, _>("starknet_estimateFee", rpc_params![]).await.unwrap()
        });
        assert!(tokio::time::timeout(Duration::from_millis(50), started_rx.recv()).await.is_err());

        let block_number = tokio::time::timeout(
            Duration::from_secs(1),
            connection_b.request::<u64, _>("starknet_blockNumber", rpc_params![]),
        )
        .await
        .expect("light request should not wait for an execution permit")
        .unwrap();
        assert_eq!(block_number, 1);

        release.add_permits(1);
        assert_eq!(first.await.unwrap(), "ok");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
                .await
                .expect("second connection should acquire the shared execution permit"),
            Some("starknet_estimateFee")
        );

        release.add_permits(1);
        assert_eq!(second.await.unwrap(), "ok");
        drop(connection_a);
        drop(connection_b);
        ctx.cancel_global();
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("server should stop after cancellation")
            .unwrap()
            .unwrap();
    }
}
