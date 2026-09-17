//! Dedicated public surface. Never nest the normal orchestrator router here.
use crate::core::config::Config;
use crate::types::{
    constant::ORCHESTRATOR_VERSION,
    jobs::{
        metadata::{SignatureCollectionMetadata, SignatureReceipt},
        types::{JobStatus, JobType},
    },
};
use crate::worker::utils::fetch_da_segment;
use alloy::primitives::{keccak256, Address, B256};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use kzg_attestation_protocol::{
    recover_signer, signing_digest, validate_request, Attestation, AttestationRequest, WorkData, WorkOffer, WorkPage,
};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use uuid::Uuid;

type ApiResult<T> = Result<T, StatusCode>;

struct ApiState {
    config: Arc<Config>,
    token_hash: B256,
    requests: Semaphore,
}

pub(crate) fn router(config: Arc<Config>, token: &str) -> Router {
    let state = Arc::new(ApiState { config, token_hash: keccak256(token), requests: Semaphore::new(16) });
    Router::new()
        .route("/v1/attestations/work", get(list_work))
        .route("/v1/attestations/work/:job_id", get(fetch_work))
        .route("/v1/attestations/work/:job_id/signatures", post(submit_signature))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .route("/healthz", get(|| async { StatusCode::OK }))
        .with_state(state)
}

async fn authenticate(State(state): State<Arc<ApiState>>, request: Request, next: Next) -> ApiResult<Response> {
    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let difference = keccak256(token).iter().zip(state.token_hash.iter()).fold(0u8, |d, (a, b)| d | (a ^ b));
    if difference != 0 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _permit = state.requests.try_acquire().map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    tokio::time::timeout(Duration::from_secs(30), next.run(request)).await.map_err(|_| StatusCode::GATEWAY_TIMEOUT)
}

fn internal(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(error = %error, "Attestation API operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkQuery {
    signer: Address,
    #[serde(default)]
    after: u64,
}

async fn list_work(State(state): State<Arc<ApiState>>, Query(query): Query<WorkQuery>) -> ApiResult<Json<WorkPage>> {
    let policy = &state.config.params.blob_attestation.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?.policy;
    if !policy.members.contains(&query.signer) {
        return Err(StatusCode::FORBIDDEN);
    }
    let jobs =
        state.config.database().get_signature_work(query.after, 32, ORCHESTRATOR_VERSION).await.map_err(internal)?;
    // Cursor follows the raw page, including work already signed, so filtering cannot starve later work.
    let next_cursor = if jobs.len() == 32 { jobs.last().map(|job| job.internal_id) } else { None };
    let mut offers = Vec::new();
    for job in jobs {
        let metadata: SignatureCollectionMetadata = job.metadata.specific.try_into().map_err(internal)?;
        if &metadata.policy != policy {
            continue;
        }
        let digest = metadata.digest.ok_or_else(|| internal("Published job has no digest"))?;
        let job_id = job.id.to_string();
        let receipts = state.config.database().get_signature_receipts(&job_id, &digest).await.map_err(internal)?;
        if receipts.iter().any(|receipt| receipt.signer == query.signer) {
            continue;
        }
        offers.push(WorkOffer { job_id, batch: job.internal_id, digest, committee_epoch: policy.committee_epoch });
    }
    Ok(Json(WorkPage { jobs: offers, next_cursor }))
}

async fn published_work(state: &ApiState, id: Uuid) -> ApiResult<SignatureCollectionMetadata> {
    let job = state.config.database().get_job_by_id(id).await.map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    if job.job_type != JobType::SignatureCollection || job.metadata.common.orchestrator_version != ORCHESTRATOR_VERSION
    {
        return Err(StatusCode::NOT_FOUND);
    }
    if job.status != JobStatus::PendingVerification {
        return Err(StatusCode::CONFLICT);
    }
    let metadata: SignatureCollectionMetadata = job.metadata.specific.try_into().map_err(internal)?;
    let configured = state.config.params.blob_attestation.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if metadata.policy != configured.policy {
        return Err(StatusCode::CONFLICT);
    }
    if metadata.digest != Some(signing_digest(&metadata.program_output, &metadata.policy)) {
        return Err(internal("Published digest mismatch"));
    }
    Ok(metadata)
}

async fn fetch_work(State(state): State<Arc<ApiState>>, Path(id): Path<Uuid>) -> ApiResult<Json<WorkData>> {
    let metadata = published_work(&state, id).await?;
    let blobs =
        fetch_da_segment(state.config.clone(), &Some(metadata.aggregator.da_segment_path)).await.map_err(internal)?;
    let request = AttestationRequest {
        chain_id: metadata.policy.chain_id,
        core_address: metadata.policy.core_address,
        committee_epoch: metadata.policy.committee_epoch,
        program_output: metadata.program_output,
        blobs: blobs.into_iter().map(Into::into).collect(),
    };
    validate_request(&request, &metadata.policy).map_err(internal)?;
    Ok(Json(WorkData { job_id: id.to_string(), request }))
}

async fn submit_signature(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<Uuid>,
    Json(attestation): Json<Attestation>,
) -> ApiResult<StatusCode> {
    let metadata = published_work(&state, id).await?;
    if Some(attestation.digest) != metadata.digest || attestation.committee_epoch != metadata.policy.committee_epoch {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let signer =
        recover_signer(attestation.digest, &attestation.signature).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    if signer != attestation.signer || !metadata.policy.members.contains(&signer) {
        return Err(StatusCode::FORBIDDEN);
    }
    let job_id = id.to_string();
    state
        .config
        .database()
        .store_signature_receipt(SignatureReceipt {
            id: format!("{job_id}:{}:{signer}", attestation.digest),
            job_id,
            digest: attestation.digest,
            signer,
            signature: attestation.signature,
        })
        .await
        .map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}
