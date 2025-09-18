use super::handler::{
    handle_add_transaction, handle_get_block, handle_get_block_bouncer_config, handle_get_block_traces,
    handle_get_class_by_hash, handle_get_compiled_class_by_class_hash, handle_get_contract_addresses,
    handle_get_public_key, handle_get_signature, handle_get_state_update,
};
use super::helpers::{not_found_response, service_unavailable_response};
use crate::handler::{handle_add_validated_transaction, handle_get_preconfirmed_block};
use crate::service::GatewayServerConfig;
use hyper::{body::Incoming, Method, Request, Response};
use mc_db::MadaraBackend;
use mc_submit_tx::{SubmitTransaction, SubmitValidatedTransaction};
use mp_utils::service::ServiceContext;
use std::{convert::Infallible, sync::Arc};

// Main router to redirect to the appropriate sub-router
pub(crate) async fn main_router(
    req: Request<Incoming>,
    backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    submit_validated: Option<Arc<dyn SubmitValidatedTransaction>>,
    ctx: ServiceContext,
    config: GatewayServerConfig,
) -> Result<Response<String>, Infallible> {
    let path = req.uri().path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>().join("/");
    match (path.as_ref(), config.feeder_gateway_enable, config.gateway_enable) {
        ("health", _, _) => Ok(Response::new("OK".to_string())),
        (path, true, _) if path.starts_with("gateway/") => {
            Ok(gateway_router(req, path, add_transaction_provider).await?)
        }
        (path, true, _) if path.starts_with("feeder_gateway/") => {
            Ok(feeder_gateway_router(req, path, backend, add_transaction_provider, ctx).await?)
        }
        (path, _, true)
            if path.starts_with("madara/trusted_add_validated_transaction")
                && config.enable_trusted_add_validated_transaction =>
        {
            Ok(handle_add_validated_transaction(req, submit_validated).await.unwrap_or_else(Into::into))
        }
        (path, false, _) if path.starts_with("feeder_gateway/") => Ok(service_unavailable_response("Feeder Gateway")),
        (path, _, false) if path.starts_with("gateway/") => Ok(service_unavailable_response("Feeder")),
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
    add_transaction_provider: Arc<dyn SubmitTransaction>,
    ctx: ServiceContext,
) -> Result<Response<String>, Infallible> {
    match (req.method(), path) {
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
        (&Method::GET, "feeder_gateway/get_block_traces") => {
            Ok(handle_get_block_traces(req, backend, add_transaction_provider, ctx).await.unwrap_or_else(Into::into))
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
    add_transaction_provider: Arc<dyn SubmitTransaction>,
) -> Result<Response<String>, Infallible> {
    match (req.method(), path) {
        (&Method::POST, "gateway/add_transaction") => {
            Ok(handle_add_transaction(req, add_transaction_provider).await.unwrap_or_else(Into::into))
        }
        _ => {
            tracing::debug!(target: "feeder_gateway", "Gateway received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}
