use std::{convert::Infallible, sync::Arc};

use hyper::{Body, Method, Request, Response};
use mc_db::MadaraBackend;
use mc_rpc::providers::AddTransactionProvider;

use super::handler::{handle_add_transaction, handle_get_block, handle_get_class_by_hash, handle_get_state_update};
use super::helpers::{not_found_response, service_unavailable_response};

// Main router to redirect to the appropriate sub-router
pub(crate) async fn main_router(
    req: Request<Body>,
    backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
) -> Result<Response<Body>, Infallible> {
    match (req.uri().path(), feeder_gateway_enable, gateway_enable) {
        ("/health", _, _) => Ok(Response::new(Body::from("OK"))),
        (path, true, _) if path.starts_with("/feeder_gateway/") => feeder_gateway_router(req, backend).await,
        (path, _, true) if path.starts_with("/feeder/") => gateway_router(req, backend, add_transaction_provider).await,
        (path, false, _) if path.starts_with("/feeder_gateway/") => Ok(service_unavailable_response("Feeder Gateway")),
        (path, _, false) if path.starts_with("/feeder/") => Ok(service_unavailable_response("Feeder")),
        _ => Ok(not_found_response()),
    }
}

// Router for requests related to feeder_gateway
async fn feeder_gateway_router(req: Request<Body>, backend: Arc<MadaraBackend>) -> Result<Response<Body>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/feeder_gateway/get_block") => Ok(handle_get_block(req, backend).await),
        (&Method::GET, "/feeder_gateway/get_state_update") => Ok(handle_get_state_update(req, backend).await),
        (&Method::GET, "/feeder_gateway/get_class_by_hash") => Ok(handle_get_class_by_hash(req, backend).await),
        _ => Ok(not_found_response()),
    }
}

// Router for requests related to feeder
async fn gateway_router(
    req: Request<Body>,
    backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Result<Response<Body>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::POST, "/feeder/add_transaction") => {
            Ok(handle_add_transaction(req, backend, add_transaction_provider).await)
        }
        _ => Ok(not_found_response()),
    }
}
