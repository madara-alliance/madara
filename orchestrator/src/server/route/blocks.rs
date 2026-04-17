use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use tracing::{error, instrument};

use super::super::types::{
    ApiResponse, BlockRouteResult, BlockSettlementStatusResponse, BlockStatusResponse,
    SettlementAggregatorBatchResponse, SettlementJobResponseItem, SettlementJobTimestampsResponse,
    SettlementSnosBatchResponse,
};
use crate::core::config::Config;
use crate::server::error::BlockRouteError;
use crate::types::batch::{AggregatorBatch, SnosBatch, SnosBatchStatus};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::{JobStatus, JobType};

#[instrument(skip(config), fields(block_number = %block_number))]
async fn handle_block_to_batch(Path(block_number): Path<u64>, State(config): State<Arc<Config>>) -> BlockRouteResult {
    match config.database().get_aggregator_batch_for_block(block_number).await {
        Ok(Some(batch)) => Ok(Json(ApiResponse::<BlockStatusResponse>::success_with_data(
            BlockStatusResponse { batch_number: batch.index },
            Some(format!("Successfully fetched batch for block {}", block_number)),
        ))
        .into_response()),
        Ok(None) => {
            tracing::warn!("No batch found for block {}, skipping for now", block_number);
            Err(BlockRouteError::NotFound(format!("No batch found for block {}", block_number)))
        }
        Err(e) => {
            error!(error = %e, "Failed to fetch batch for block");
            Err(BlockRouteError::DatabaseError(e.to_string()))
        }
    }
}

#[instrument(skip(config), fields(block_number = %block_number))]
async fn handle_block_settlement_status(
    Path(block_number): Path<u64>,
    State(config): State<Arc<Config>>,
) -> BlockRouteResult {
    let block_lookup = config
        .database()
        .get_block_batch_lookup(block_number)
        .await
        .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

    let mut block_jobs = config
        .database()
        .get_jobs_by_block_number(block_number)
        .await
        .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

    let aggregator_batch = match block_lookup.as_ref().and_then(|lookup| lookup.aggregator_batch_index) {
        Some(aggregator_batch_index) => config
            .database()
            .get_aggregator_batches_by_indexes(vec![aggregator_batch_index])
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
            .into_iter()
            .next(),
        None => config
            .database()
            .get_aggregator_batch_for_block(block_number)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?,
    };

    let block_snos_batch = match block_lookup.as_ref().and_then(|lookup| lookup.snos_batch_index) {
        Some(snos_batch_index) => config
            .database()
            .get_snos_batches_by_indices(vec![snos_batch_index])
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
            .into_iter()
            .next(),
        None => None,
    };

    let mut snos_batch_response = None;
    let mut aggregator_proof_jobs = Vec::new();

    if let Some(batch) = &aggregator_batch {
        let snos_batches = config
            .database()
            .get_snos_batches_by_aggregator_index(batch.index)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?;

        let matching_snos_batch = block_snos_batch.as_ref().or_else(|| {
            snos_batches
                .iter()
                .find(|snos_batch| snos_batch.start_block <= block_number && snos_batch.end_block >= block_number)
        });

        let proof_jobs_by_internal_id = config
            .database()
            .get_jobs_by_internal_ids_and_type(
                snos_batches.iter().map(|snos_batch| snos_batch.index).collect(),
                &JobType::ProofCreation,
            )
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
            .into_iter()
            .map(|job| (job.internal_id, job))
            .collect::<HashMap<_, _>>();

        if let Some(block_snos_batch) = matching_snos_batch {
            snos_batch_response = Some(snapshot_snos_batch(block_snos_batch));

            if let Some(proof_job) = proof_jobs_by_internal_id.get(&block_snos_batch.index).cloned() {
                push_job_if_missing(&mut block_jobs, proof_job);
            }
        }

        if let Some(aggregator_job) = config
            .database()
            .get_job_by_internal_id_and_type(batch.index, &JobType::Aggregator)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
        {
            push_job_if_missing(&mut block_jobs, aggregator_job);
        }

        if let Some(state_transition_job) = config
            .database()
            .get_job_by_internal_id_and_type(batch.index, &JobType::StateTransition)
            .await
            .map_err(|e| BlockRouteError::DatabaseError(e.to_string()))?
        {
            push_job_if_missing(&mut block_jobs, state_transition_job);
        }

        for snos_batch in &snos_batches {
            if let Some(job) = proof_jobs_by_internal_id.get(&snos_batch.index) {
                aggregator_proof_jobs.push(snapshot_job(job));
            }
        }
    } else {
        snos_batch_response = block_jobs.iter().find_map(|job| match &job.metadata.specific {
            JobSpecificMetadata::Snos(metadata) => Some(SettlementSnosBatchResponse {
                index: metadata.snos_batch_index,
                aggregator_batch_index: None,
                start_block: metadata.start_block,
                end_block: metadata.end_block,
                status: derive_snos_batch_status_from_job(job.status.clone()),
                created_at: job.created_at,
                updated_at: job.updated_at,
            }),
            _ => None,
        });
    }

    Ok(Json(ApiResponse::<BlockSettlementStatusResponse>::success_with_data(
        BlockSettlementStatusResponse {
            block_number,
            snos_batch: snos_batch_response,
            aggregator_batch: aggregator_batch.as_ref().map(snapshot_aggregator_batch),
            block_jobs: block_jobs.iter().map(snapshot_job).collect(),
            aggregator_proof_jobs,
        },
        Some(format!("Successfully fetched settlement status for block {}", block_number)),
    ))
    .into_response())
}

fn push_job_if_missing(jobs: &mut Vec<JobItem>, job: JobItem) {
    if jobs.iter().any(|existing| existing.id == job.id) {
        return;
    }
    jobs.push(job);
}

fn snapshot_job(job: &JobItem) -> SettlementJobResponseItem {
    SettlementJobResponseItem {
        job_type: job.job_type.clone(),
        id: job.id,
        internal_id: job.internal_id,
        status: job.status.clone(),
        created_at: job.created_at,
        updated_at: job.updated_at,
        timestamps: SettlementJobTimestampsResponse {
            process_started_at: job.metadata.common.process_started_at,
            process_completed_at: job.metadata.common.process_completed_at,
            verification_started_at: job.metadata.common.verification_started_at,
            verification_completed_at: job.metadata.common.verification_completed_at,
        },
    }
}

fn snapshot_snos_batch(batch: &SnosBatch) -> SettlementSnosBatchResponse {
    SettlementSnosBatchResponse {
        index: batch.index,
        aggregator_batch_index: batch.aggregator_batch_index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
    }
}

fn snapshot_aggregator_batch(batch: &AggregatorBatch) -> SettlementAggregatorBatchResponse {
    SettlementAggregatorBatchResponse {
        index: batch.index,
        start_block: batch.start_block,
        end_block: batch.end_block,
        status: batch.status.clone(),
        created_at: batch.created_at,
        updated_at: batch.updated_at,
    }
}

fn derive_snos_batch_status_from_job(job_status: JobStatus) -> SnosBatchStatus {
    match job_status {
        JobStatus::Completed => SnosBatchStatus::Completed,
        _ => SnosBatchStatus::SnosJobCreated,
    }
}

pub(super) fn block_router(config: Arc<Config>) -> Router {
    Router::new()
        .route("/batch-for-block/:block_number", get(handle_block_to_batch))
        .route("/settlement-status/:block_number", get(handle_block_settlement_status))
        .with_state(config)
}
