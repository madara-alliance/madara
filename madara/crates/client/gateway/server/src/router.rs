use super::handler::{
    handle_add_transaction, handle_get_block, handle_get_block_bouncer_config, handle_get_block_hash_by_id,
    handle_get_block_id_by_hash, handle_get_block_traces, handle_get_class_by_hash,
    handle_get_compiled_class_by_class_hash, handle_get_contract_addresses, handle_get_public_key,
    handle_get_signature, handle_get_state_update, handle_get_transaction, handle_get_transaction_status,
};
use super::helpers::{not_found_response, service_unavailable_response};
use crate::handler::{handle_add_validated_transaction, handle_get_preconfirmed_block};
use crate::service::GatewayServerConfig;
use hyper::{body::Incoming, Method, Request, Response};
use mc_db::MadaraBackend;
use mc_submit_tx::{SubmitTransaction, SubmitValidatedTransaction, TransactionLookup};
use std::{convert::Infallible, sync::Arc};

// Main router to redirect to the appropriate sub-router
pub(crate) async fn main_router(
    req: Request<Incoming>,
    path: &str,
    backend: Arc<MadaraBackend>,
    transaction_submitter: Arc<dyn SubmitTransaction>,
    transaction_lookup: Arc<dyn TransactionLookup>,
    submit_validated: Option<Arc<dyn SubmitValidatedTransaction>>,
    config: GatewayServerConfig,
) -> Result<Response<String>, Infallible> {
    match (path, config.feeder_gateway_enable, config.gateway_enable) {
        ("health", _, _) => Ok(Response::new("OK".to_string())),
        (path, _, true) if path.starts_with("gateway/") => Ok(gateway_router(req, path, transaction_submitter).await?),
        (path, true, _) if path.starts_with("feeder_gateway/") => {
            Ok(feeder_gateway_router(req, path, backend, transaction_lookup).await?)
        }
        (path, _, true)
            if path.starts_with("madara/trusted_add_validated_transaction")
                && config.enable_trusted_add_validated_transaction =>
        {
            Ok(handle_add_validated_transaction(req, submit_validated).await.unwrap_or_else(Into::into))
        }
        (path, false, _) if path.starts_with("feeder_gateway/") => Ok(service_unavailable_response("Feeder Gateway")),
        (path, _, false) if path.starts_with("gateway/") => Ok(service_unavailable_response("Gateway")),
        _ => {
            tracing::debug!(target: "feeder_gateway", "Main router received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}

// Router for requests related to feeder_gateway
async fn feeder_gateway_router(
    req: Request<Incoming>,
    path: &str,
    backend: Arc<MadaraBackend>,
    transaction_lookup: Arc<dyn TransactionLookup>,
) -> Result<Response<String>, Infallible> {
    match (req.method(), path) {
        (&Method::GET, "feeder_gateway/get_committed_data_roots") => {
            Ok(crate::committed_data::get(req, backend, "roots").await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_committed_data") => {
            Ok(crate::committed_data::get(req, backend, "page").await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_committed_data_witness") => {
            Ok(crate::committed_data::get(req, backend, "witness").await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_preconfirmed_block") => {
            Ok(handle_get_preconfirmed_block(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_block") => {
            Ok(handle_get_block(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_signature") => {
            Ok(handle_get_signature(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_state_update") => {
            Ok(handle_get_state_update(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_transaction") => {
            Ok(handle_get_transaction(req, backend, transaction_lookup).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_transaction_status") => {
            Ok(handle_get_transaction_status(req, backend, transaction_lookup).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_block_hash_by_id") => {
            Ok(handle_get_block_hash_by_id(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_block_id_by_hash") => {
            Ok(handle_get_block_id_by_hash(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_block_traces") => {
            Ok(handle_get_block_traces(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_class_by_hash") => {
            Ok(handle_get_class_by_hash(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_compiled_class_by_class_hash") => {
            Ok(handle_get_compiled_class_by_class_hash(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_contract_addresses") => {
            Ok(handle_get_contract_addresses(backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_public_key") => {
            Ok(handle_get_public_key(backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_block_bouncer_weights") => {
            Ok(handle_get_block_bouncer_config(req, backend).await.unwrap_or_else(Into::into))
        }
        _ => {
            tracing::debug!(target: "feeder_gateway", "Feeder gateway received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}

// Router for requests related to feeder
async fn gateway_router(
    req: Request<Incoming>,
    path: &str,
    transaction_submitter: Arc<dyn SubmitTransaction>,
) -> Result<Response<String>, Infallible> {
    match (req.method(), path) {
        (&Method::POST, "gateway/add_transaction") => {
            Ok(handle_add_transaction(req, transaction_submitter).await.unwrap_or_else(Into::into))
        }
        _ => {
            tracing::debug!(target: "feeder_gateway", "Gateway received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}

#[cfg(test)]
mod committed_data_tests {
    use super::*;
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    use starknet_types_core::felt::Felt;

    struct NoTransactions;
    #[async_trait::async_trait]
    impl TransactionLookup for NoTransactions {
        async fn received_transaction(&self, _: Felt) -> Option<bool> {
            None
        }
        async fn subscribe_new_transactions(&self) -> Option<tokio::sync::broadcast::Receiver<Felt>> {
            None
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn committed_data_feeder_serves_gets_and_rejects_post_and_invalid_offsets() {
        use std::io::{Read, Write};
        let backend = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
        let values = vec![Felt::ONE, Felt::TWO];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..5 {
                let (stream, _) = listener.accept().await.unwrap();
                let backend = backend.clone();
                let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let backend = backend.clone();
                    async move {
                        let path = req.uri().path().trim_start_matches('/').to_string();
                        let response =
                            feeder_gateway_router(req, &path, backend, Arc::new(NoTransactions)).await.unwrap();
                        Ok::<_, Infallible>(response.map(|body| http_body_util::Full::new(bytes::Bytes::from(body))))
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await
                    .unwrap();
            }
        });
        let query = move |method: &'static str, path: String| async move {
            tokio::task::spawn_blocking(move || {
                let mut stream = std::net::TcpStream::connect(addr).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                write!(stream, "{method} /feeder_gateway/{path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                let (head, body) = response.split_once("\r\n\r\n").unwrap();
                (head.to_string(), body.to_string())
            }).await.unwrap()
        };
        let (head, body) = query("GET", "get_committed_data_roots".into()).await;
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(serde_json::from_str::<Vec<Felt>>(&body).unwrap(), vec![root]);
        let (_, body) = query("GET", format!("get_committed_data?root={root:#x}&start=0")).await;
        let page: mp_gateway::committed_data::CommittedDataPage = serde_json::from_str(&body).unwrap();
        assert_eq!(page.values, values);
        let (_, body) = query("GET", format!("get_committed_data_witness?root={root:#x}&index=1")).await;
        let witness: blockifier::execution::syscalls::committed_data::CommittedDataWitness =
            serde_json::from_str(&body).unwrap();
        assert!(witness.verify());
        assert!(query("GET", format!("get_committed_data?root={root:#x}&start=1")).await.0.starts_with("HTTP/1.1 400"));
        assert!(query("POST", "get_committed_data".into()).await.0.starts_with("HTTP/1.1 404"));
        server.await.unwrap();
    }
}
