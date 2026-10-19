use std::sync::Arc;

use orchestrator_utils::layer::Layer;

use crate::core::client::database::{AggregatorBatchDbQuery, SnosBatchDbQuery};
use crate::core::config::Config;
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::JobType;

struct JobAlertContext {
    batch_label: &'static str,
    block_label: &'static str,
    block_range: Option<(u64, u64)>,
}

async fn resolve_job_alert_context(job: &JobItem, config: &Arc<Config>) -> JobAlertContext {
    let block_label = match config.layer() {
        Layer::L2 => "L2 block",
        Layer::L3 => "L3 block",
    };

    let uses_aggregator_batch = job.job_type == JobType::Aggregator
        || (job.job_type == JobType::StateTransition && config.layer() == &Layer::L2);

    if uses_aggregator_batch {
        let block_range = match config
            .database()
            .get_aggregator_batches(AggregatorBatchDbQuery {
                indexes: Some(vec![job.internal_id]),
                ..Default::default()
            })
            .await
        {
            Ok(batches) => batches.first().map(|batch| (batch.start_block, batch.end_block)),
            Err(error) => {
                tracing::warn!(
                    job_id = %job.id,
                    internal_id = job.internal_id,
                    error = %error,
                    "Failed to resolve aggregator batch block range for alert"
                );
                None
            }
        };

        return JobAlertContext { batch_label: "Aggregator batch ID", block_label, block_range };
    }

    // SNOS metadata already carries the exact range. Descendant jobs retain the
    // SNOS batch index as their internal ID, so resolve those through the batch table.
    let metadata_range = match &job.metadata.specific {
        JobSpecificMetadata::Snos(metadata) => Some((metadata.start_block, metadata.end_block)),
        _ => None,
    };
    let block_range = if metadata_range.is_some() {
        metadata_range
    } else {
        match config
            .database()
            .get_snos_batches(SnosBatchDbQuery { indexes: Some(vec![job.internal_id]), ..Default::default() })
            .await
        {
            Ok(batches) => batches.first().map(|batch| (batch.start_block, batch.end_block)),
            Err(error) => {
                tracing::warn!(
                    job_id = %job.id,
                    internal_id = job.internal_id,
                    error = %error,
                    "Failed to resolve SNOS batch block range for alert"
                );
                None
            }
        }
    };

    JobAlertContext { batch_label: "SNOS batch ID", block_label, block_range }
}

fn external_task_id(job: &JobItem) -> Option<String> {
    match &job.external_id {
        ExternalId::String(id) if !id.is_empty() => Some(id.to_string()),
        ExternalId::Number(id) if *id != 0 => Some(id.to_string()),
        _ => None,
    }
}

pub(crate) async fn format_job_alert_context(job: &JobItem, config: &Arc<Config>) -> String {
    let context = resolve_job_alert_context(job, config).await;
    let block_context = match context.block_range {
        Some((start, end)) if start == end => format!("{}: {}", context.block_label, start),
        Some((start, end)) => format!("{}s: {}-{}", context.block_label, start, end),
        None => format!("{} range: unavailable", context.block_label),
    };
    let task_context = match (&job.job_type, external_task_id(job)) {
        (JobType::ProofCreation | JobType::ProofRegistration, Some(task_id)) => {
            format!("\nProver task ID: {}", task_id)
        }
        (JobType::Aggregator, Some(task_id)) => format!("\nProver aggregation task ID: {}", task_id),
        _ => String::new(),
    };

    format!(
        "Chain ID: {}\n\
         Layer: {:?}\n\
         Prover: {}\n\
         Job ID: {}\n\
         Type: {:?}\n\
         {}: {}\n\
         {}{}",
        config.chain_details().chain_id,
        config.layer(),
        config.prover_kind().label(),
        job.id,
        job.job_type,
        context.batch_label,
        job.internal_id,
        block_context,
        task_context
    )
}

pub(crate) async fn verification_timeout_alert_message(job: &JobItem, config: &Arc<Config>) -> String {
    format!(
        "Job Verification Timeout Alert\n{}\nVerification attempts: {}",
        format_job_alert_context(job, config).await,
        job.metadata.common.verification_attempt_no
    )
}
