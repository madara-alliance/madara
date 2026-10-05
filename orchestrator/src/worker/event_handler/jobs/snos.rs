use crate::core::config::Config;
use crate::core::StorageClient;
use crate::error::job::fact::FactError;
use crate::error::job::snos::SnosError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::params::snos::SNOSParams;
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::fact_info::{get_fact_info, get_fact_l2, get_program_output};
use async_trait::async_trait;
use cairo_vm::Felt252;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use generate_pie::error::PieGenerationError;
use generate_pie::types::chain_config::ChainConfig;
use generate_pie::types::os_hints::OsHintsConfiguration;
use generate_pie::types::pie::{PieGenerationInput, PieGenerationTiming};
use generate_pie::{execute_prepared_pie, prepare_pie, prepare_pie_from_witness, PreparedPieGeneration, RpcWitness};
use orchestrator_utils::chain_details::ChainDetails;
use orchestrator_utils::layer::Layer;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet_core::types::Felt;
use url::Url;

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn, Span};

/// Delay before retrying when SNOS RPC is unavailable (in seconds)
const SNOS_UNAVAILABLE_RETRY_DELAY_SECS: u64 = 60;

/// Only one job per process may own a full CairoPIE/finalization footprint.
static SNOS_FINALIZATION_SEMAPHORE: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
static SNOS_WITNESS_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

struct FinalizedSnosOutput {
    timing: PieGenerationTiming,
    cairo_pie_zip_bytes: bytes::Bytes,
    snos_output: Vec<Felt>,
    program_output: Vec<Felt252>,
    fact_hash: String,
    n_steps: usize,
}

/// Check if SNOS RPC is healthy by calling chain_id.
/// Returns true if the RPC is reachable and responds, false otherwise.
pub async fn check_snos_health(snos_url: &Url) -> bool {
    let provider = JsonRpcClient::new(HttpTransport::new(snos_url.clone()));
    provider.chain_id().await.is_ok()
}

pub(crate) fn rpc_for_snos_attempt<'a>(snos_config: &'a SNOSParams, job: &JobItem) -> &'a Url {
    match (should_use_snos_backup_rpc(snos_config, job), snos_config.rpc_for_snos_backup.as_ref()) {
        (true, Some(backup_rpc)) => backup_rpc,
        _ => &snos_config.rpc_for_snos,
    }
}

pub(crate) fn should_use_snos_backup_rpc(snos_config: &SNOSParams, job: &JobItem) -> bool {
    job.metadata.common.process_retry_attempt_no > 0 && snos_config.rpc_for_snos_backup.is_some()
}

pub struct SnosJobHandler;

/// Extension trait to convert ChainDetails to ChainConfig for SNOS usage.
pub trait ChainDetailsExt {
    /// Convert ChainDetails to the ChainConfig type required by generate_pie.
    fn to_chain_config(&self) -> ChainConfig;
}

impl ChainDetailsExt for ChainDetails {
    fn to_chain_config(&self) -> ChainConfig {
        ChainConfig::new(&self.chain_id, &self.strk_fee_token_address, &self.eth_fee_token_address, self.is_l3)
    }
}

trait OsHintsConfigurationFromLayer {
    fn with_layer(layer: Layer) -> OsHintsConfiguration;
}

impl OsHintsConfigurationFromLayer for OsHintsConfiguration {
    fn with_layer(layer: Layer) -> OsHintsConfiguration {
        match layer {
            Layer::L2 => OsHintsConfiguration { debug_mode: true, full_output: true, use_kzg_da: false },
            Layer::L3 => OsHintsConfiguration { debug_mode: true, full_output: false, use_kzg_da: true },
        }
    }
}

#[async_trait]
impl JobHandlerTrait for SnosJobHandler {
    async fn create_job(&self, internal_id: u64, metadata: JobMetadata) -> Result<JobItem, JobError> {
        debug!(log_type = "starting", "{:?} job {} creation started", JobType::SnosRun, internal_id);
        let job_item = JobItem::create(internal_id, JobType::SnosRun, JobStatus::Created, metadata);
        debug!(log_type = "completed", "{:?} job {} creation completed", JobType::SnosRun, internal_id);
        Ok(job_item)
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id;
        info!(log_type = "starting", job_id = %job.id, " {:?} job {} processing started", JobType::SnosRun, internal_id);
        let start_time = Instant::now();

        // Get SNOS metadata
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(error = %e, "Failed to convert metadata to SnosMetadata");
        })?;

        debug!("SNOS metadata retrieved {:?}", snos_metadata);

        // Get block number from metadata (using start_block as the primary block for processing)
        let start_block_number = snos_metadata.start_block;
        let end_block_number = snos_metadata.end_block;
        debug!(start_block = %snos_metadata.start_block, end_block = %snos_metadata.end_block, num_blocks = %snos_metadata.num_blocks, "Retrieved batch information from metadata");

        if should_use_snos_backup_rpc(config.snos_config(), job) {
            info!(
                job_id = %job.id,
                internal_id,
                retry_attempt = job.metadata.common.process_retry_attempt_no,
                "Using backup SNOS RPC for retried job"
            );
            MetricsRecorder::record_snos_rpc_fallback(job);
        }

        let snos_url = rpc_for_snos_attempt(config.snos_config(), job).to_string();
        let snos_url = snos_url.trim_end_matches('/');
        debug!("Calling generate_pie function");

        // Get DA public keys (already parsed as Felt values in config)
        let public_keys: Option<Vec<Felt>> = config.da_public_keys().cloned();

        let input = PieGenerationInput {
            rpc_url: snos_url.to_string(),
            blocks: (start_block_number..=end_block_number).collect(),
            // Use chain details fetched at orchestrator startup (no RPC call needed here)
            chain_config: config.chain_details().to_chain_config(),
            os_hints_config: OsHintsConfiguration::with_layer(config.layer().clone()),
            output_path: None, // No file output
            layout: config.params.snos_layout_name,
            versioned_constants: config.snos_config().versioned_constants.clone(),
            public_keys,
        };

        let (prepared, witness_fetch_time_ms, witness_response_count) =
            prepare_snos(input, config.snos_config().snos_witness_url.as_ref()).await.map_err(|e| {
                error!(error = %e, "SNOS preparation failed");
                SnosError::SnosExecutionError { internal_id, source: e }
            })?;
        let finalized =
            run_in_snos_finalization_lane(move || finalize_snos(prepared, internal_id)).await.map_err(|source| {
                SnosError::SnosExecutionError {
                    internal_id,
                    source: generate_pie::error::PieGenerationError::TaskJoin(source),
                }
            })??;
        debug!("SNOS finalization completed successfully");

        let FinalizedSnosOutput { timing, cairo_pie_zip_bytes, snos_output, program_output, fact_hash, n_steps } =
            finalized;
        let PieGenerationTiming {
            total_processing_time_ms,
            rpc_wait_time_ms,
            execution_time_ms,
            finalization_wait_time_ms,
            rpc_calls_by_method,
        } = timing;

        // Update the metadata with new paths and fact info
        if let JobSpecificMetadata::Snos(metadata) = &mut job.metadata.specific {
            metadata.snos_fact = Some(fact_hash);
            metadata.snos_n_steps = Some(n_steps);
            metadata.snos_total_processing_time_ms = Some(total_processing_time_ms);
            metadata.snos_rpc_wait_time_ms = Some(rpc_wait_time_ms);
            metadata.snos_execution_time_ms = Some(execution_time_ms);
            metadata.snos_finalization_wait_time_ms = Some(finalization_wait_time_ms);
            metadata.snos_rpc_calls_by_method = Some(rpc_calls_by_method);
            metadata.snos_witness_fetch_time_ms = witness_fetch_time_ms;
            metadata.snos_witness_response_count = witness_response_count;
        }

        debug!("Storing SNOS outputs");
        self.store(internal_id, config.storage(), &snos_metadata, cairo_pie_zip_bytes, snos_output, program_output)
            .await?;

        MetricsRecorder::record_snos_job_processing_time(start_time.elapsed().as_secs_f64());
        info!(log_type = "completed", job_id = %job.id, "{:?} job {} processed successfully", JobType::SnosRun, internal_id);

        Ok(snos_metadata.snos_batch_index.to_string())
    }

    async fn verify_job(&self, _config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::SnosRun, internal_id);
        // No need for verification as of now. If we later on decide to outsource SNOS run
        // to another service, verify_job can be used to poll on the status of the job
        info!(log_type = "completed", job_id = %job.id, "{:?} job {} verification completed", JobType::SnosRun, internal_id);
        Ok(JobVerificationStatus::Verified)
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        1
    }

    async fn check_ready_to_process(&self, config: Arc<Config>, job: &JobItem) -> Result<(), Duration> {
        if let Some(witness_url) = config.snos_config().snos_witness_url.as_ref() {
            let healthy = match witness_url.join("health") {
                Ok(url) => {
                    SNOS_WITNESS_HTTP_CLIENT.get(url).send().await.is_ok_and(|response| response.status().is_success())
                }
                Err(_) => false,
            };
            if healthy {
                return Ok(());
            }
            warn!(witness_url = %witness_url, "SNOS witness service is unavailable, job will be requeued");
            return Err(Duration::from_secs(SNOS_UNAVAILABLE_RETRY_DELAY_SECS));
        }

        let snos_url = rpc_for_snos_attempt(config.snos_config(), job);

        if !check_snos_health(snos_url).await {
            // SNOS is down - signal to requeue with delay
            warn!(snos_url = %snos_url, "SNOS RPC is unavailable, job will be requeued");

            return Err(Duration::from_secs(SNOS_UNAVAILABLE_RETRY_DELAY_SECS));
        }

        Ok(())
    }
}

async fn prepare_snos(
    input: PieGenerationInput,
    witness_url: Option<&Url>,
) -> Result<(PreparedPieGeneration, Option<u64>, Option<usize>), PieGenerationError> {
    let Some(witness_url) = witness_url else {
        return Ok((prepare_pie(input).await?, None, None));
    };

    let started_at = Instant::now();
    let mut witnesses = Vec::with_capacity(input.blocks.len());
    for block_number in &input.blocks {
        let endpoint = witness_url
            .join("feeder_gateway/get_block_witness")
            .map_err(|error| PieGenerationError::RpcClient(format!("Invalid SNOS witness URL: {error}")))?;
        let witness = SNOS_WITNESS_HTTP_CLIENT
            .get(endpoint)
            .query(&[("blockNumber", block_number)])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| {
                PieGenerationError::RpcClient(format!("Failed to fetch block {block_number} witness: {error}"))
            })?
            .json::<RpcWitness>()
            .await
            .map_err(|error| {
                PieGenerationError::RpcClient(format!("Failed to decode block {block_number} witness: {error}"))
            })?;
        witnesses.push(witness);
    }
    let witness = RpcWitness::merge(witnesses)
        .map_err(|error| PieGenerationError::RpcClient(format!("Failed to merge SNOS witnesses: {error}")))?;
    let response_count = witness.response_count();
    let fetch_time_ms = started_at.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    info!(fetch_time_ms, response_count, blocks = input.blocks.len(), "SNOS witnesses loaded");
    Ok((prepare_pie_from_witness(input, witness).await?, Some(fetch_time_ms), Some(response_count)))
}

impl SnosJobHandler {
    /// Stores the [CairoPie] and the [StarknetOsOutput] in the Data Storage.
    /// The paths will be:
    ///     - [block_number]/cairo_pie.zip
    ///     - [block_number]/snos_output.json
    async fn store(
        &self,
        internal_id: u64,
        data_storage: &dyn StorageClient,
        snos_metadata: &SnosMetadata,
        cairo_pie_zip_bytes: bytes::Bytes,
        snos_output: Vec<Felt>,
        program_output: Vec<Felt252>,
    ) -> Result<(), SnosError> {
        // Get storage paths from metadata
        let cairo_pie_key = snos_metadata
            .cairo_pie_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("Cairo Pie path not found in metadata"))))?;

        let snos_output_key = snos_metadata
            .snos_output_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("SNOS output path not found in metadata"))))?;

        let program_output_key = snos_metadata
            .program_output_path
            .as_ref()
            .ok_or_else(|| SnosError::Other(OtherError(eyre!("Program output path not found in metadata"))))?;

        // Store Cairo Pie
        {
            data_storage
                .put_data(cairo_pie_zip_bytes, cairo_pie_key)
                .await
                .map_err(|source| SnosError::CairoPieUnstorable { internal_id, source })?;
        }

        // Store SNOS Output
        {
            let snos_output_json = serde_json::to_vec(&snos_output)
                .map_err(|e| SnosError::SnosOutputUnserializable { internal_id, message: e.to_string() })?;
            data_storage
                .put_data(snos_output_json.into(), snos_output_key)
                .await
                .map_err(|source| SnosError::SnosOutputUnstorable { internal_id, source })?;
        }

        // Store Program Output
        {
            let program_output: Vec<[u8; 32]> = program_output.iter().map(|f| f.to_bytes_be()).collect();
            let encoded_data = bincode::serialize(&program_output)
                .map_err(|e| SnosError::ProgramOutputUnserializable { internal_id, message: e.to_string() })?;
            data_storage
                .put_data(encoded_data.into(), program_output_key)
                .await
                .map_err(|source| SnosError::ProgramOutputUnstorable { internal_id, source })?;
        }

        Ok(())
    }
}

async fn run_in_snos_finalization_lane<F, T>(finalize: F) -> Result<T, tokio::task::JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = Arc::clone(&SNOS_FINALIZATION_SEMAPHORE)
        .acquire_owned()
        .await
        .expect("SNOS finalization semaphore must remain open");
    let span = Span::current();

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        span.in_scope(finalize)
    })
    .await
}

fn finalize_snos(prepared: PreparedPieGeneration, internal_id: u64) -> Result<FinalizedSnosOutput, JobError> {
    let snos_output = execute_prepared_pie(prepared).map_err(|source| {
        error!(error = %source, "SNOS execution failed");
        SnosError::SnosExecutionError { internal_id, source }
    })?;
    let timing = snos_output.timing;
    let output = snos_output.output;
    let cairo_pie = output.cairo_pie;
    let snos_output = output.raw_os_output;
    let n_steps = cairo_pie.execution_resources.n_steps;

    // TODO: Return a typed OS output from SNOS instead of indexing a Vec<Felt>.
    let (fact_hash, program_output) = if snos_output.get(8) == Some(&Felt::ZERO) {
        debug!("Using calldata for settlement layer");
        let fact_hash = get_fact_l2(&cairo_pie, None).map_err(|e| {
            error!(error = %e, "Failed to get fact hash");
            JobError::FactError(FactError::L2FactCompute)
        })?;
        let program_output = get_program_output(&cairo_pie, false).map_err(|e| {
            error!(error = %e, "Failed to get program output");
            JobError::FactError(FactError::ProgramOutputCompute)
        })?;
        (fact_hash, program_output)
    } else if snos_output.get(8) == Some(&Felt::ONE) {
        debug!("Using blobs for settlement layer");
        let fact_info = get_fact_info(&cairo_pie, None, false)?;
        (fact_info.fact, fact_info.program_output)
    } else {
        error!("Invalid KZG flag");
        return Err(SnosError::UnsupportedKZGFlag.into());
    };

    let cairo_pie_zip_bytes = crate::worker::utils::pie::cairo_pie_to_zip_bytes_blocking(cairo_pie)
        .map_err(|e| SnosError::CairoPieUnserializable { internal_id, message: e.to_string() })?;

    Ok(FinalizedSnosOutput {
        timing,
        cairo_pie_zip_bytes,
        snos_output,
        program_output,
        fact_hash: fact_hash.to_string(),
        n_steps,
    })
}

#[cfg(test)]
mod finalization_tests {
    use super::run_in_snos_finalization_lane;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finalization_lane_runs_only_one_closure_at_a_time() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let run = || {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            run_in_snos_finalization_lane(move || {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                active.fetch_sub(1, Ordering::SeqCst);
            })
        };

        let (first, second) = tokio::join!(run(), run());
        first.unwrap();
        second.unwrap();
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }
}
