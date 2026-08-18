use crate::core::client::lock::LockValue;
use crate::core::config::Config;
use crate::error::job::JobError;
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::triggers::batching::aggregator::{
    AggregatorBatchConfig, AggregatorHandler, AggregatorState, AggregatorStateHandler,
};
use crate::worker::event_handler::triggers::batching::replay_bounds;
use crate::worker::event_handler::triggers::batching::BlockProcessingResult;
use crate::worker::event_handler::triggers::JobTrigger;
use starknet::providers::Provider;
use std::cmp::{max, min};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info};

pub const AGGREGATOR_BATCHING_WORKER_KEY: &str = "AggregatorBatchingWorker";

pub struct AggregatorBatchingTrigger;

#[async_trait::async_trait]
impl JobTrigger for AggregatorBatchingTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Trying to acquire lock on Aggregator Batching Worker
        let worker_started_at = Instant::now();
        match config
            .lock()
            .acquire_lock(
                AGGREGATOR_BATCHING_WORKER_KEY,
                LockValue::Boolean(false),
                config.params.batching_config.batching_worker_lock_duration,
                None,
            )
            .await
        {
            Ok(_) => {
                // Lock acquired successfully
                info!(
                    lock_key = AGGREGATOR_BATCHING_WORKER_KEY,
                    lock_duration_seconds = config.params.batching_config.batching_worker_lock_duration,
                    "Aggregator batching worker acquired lock"
                );
            }
            Err(err) => {
                // Failed to acquire lock
                // Returning safely
                info!(
                    lock_key = AGGREGATOR_BATCHING_WORKER_KEY,
                    error = %err,
                    "Aggregator batching worker failed to acquire lock, returning safely"
                );
                return Ok(());
            }
        }

        let batching_handler =
            AggregatorHandler::new(config.clone(), AggregatorBatchConfig::from_config(&config.params));

        let state_handler = AggregatorStateHandler::from_config(&config);

        // Execute the main work and capture the result
        let mut batch_number = None;
        let result = async {
            let (start_block, end_block) = self.calculate_range(&config).await?;

            // If there are no blocks to process, return early
            if start_block > end_block {
                info!(start_block, end_block, "No Aggregator blocks to process");
                return Ok(());
            }

            info!("Processing Aggregator batches for blocks {} to {}", start_block, end_block);

            let mut state = state_handler.load_batch_state().await?;
            let mut replay_bounds_error: Option<replay_bounds::ReplayBoundsError> = None;

            for block_num in start_block..=end_block {
                let block_started_at = Instant::now();
                if let Some(ref_client) = config.replay_bounds_client() {
                    if let Err(e) =
                        replay_bounds::validate_block_hash(config.madara_rpc_client(), ref_client, block_num).await
                    {
                        error!(block_num, "Replay bounds: {}, stopping aggregator batching", e);
                        replay_bounds_error = Some(e);
                        break;
                    }
                }

                let block_result = match batching_handler.include_block(block_num, state).await? {
                    BlockProcessingResult::Accumulated(updated_state) => {
                        state = AggregatorState::NonEmpty(updated_state);
                        "accumulated"
                    }
                    BlockProcessingResult::BatchCompleted { completed_state, new_state } => {
                        batch_number = Some(completed_state.batch_index());
                        state_handler.save_batch_state(&completed_state).await?;
                        state = new_state;
                        "batch_completed"
                    }
                    BlockProcessingResult::NotBatched(current_state) => {
                        state = current_state;
                        "not_batched"
                    }
                };
                info!(
                    block_num,
                    duration_ms = %block_started_at.elapsed().as_millis(),
                    result = block_result,
                    "Aggregator batching worker finished block"
                );
                if block_result == "not_batched" {
                    break;
                }
            }

            // Save valid partial state before propagating the error
            match state {
                AggregatorState::Empty(_) => {}
                AggregatorState::NonEmpty(state) => {
                    batch_number = Some(state.batch_index());
                    state_handler.save_batch_state(&state).await?;
                }
            }

            if let Some(e) = replay_bounds_error {
                return Err(color_eyre::eyre::eyre!("Replay bounds validation failed: {}", e));
            }

            Ok(())
        }
        .await;

        // Always release the lock, regardless of whether work succeeded or failed
        let release_started_at = Instant::now();
        info!(lock_key = AGGREGATOR_BATCHING_WORKER_KEY, "Aggregator batching worker releasing lock");
        let release_result = config.lock().release_lock(AGGREGATOR_BATCHING_WORKER_KEY, None).await;
        if let Err(e) = &release_result {
            error!(
                lock_key = AGGREGATOR_BATCHING_WORKER_KEY,
                duration_ms = %release_started_at.elapsed().as_millis(),
                "Failed to release {} lock: {}",
                AGGREGATOR_BATCHING_WORKER_KEY,
                e
            );
        } else {
            info!(
                lock_key = AGGREGATOR_BATCHING_WORKER_KEY,
                duration_ms = %release_started_at.elapsed().as_millis(),
                worker_duration_ms = %worker_started_at.elapsed().as_millis(),
                "Aggregator batching worker released lock"
            );
        }

        MetricsRecorder::record_aggregator_batching_duration(worker_started_at.elapsed().as_secs_f64());
        if let Some(batch_number) = batch_number {
            MetricsRecorder::record_aggregator_batching_batch_number(batch_number);
        }

        // If work succeeded but lock release failed, return the lock release error.
        if result.is_ok() {
            if let Err(e) = release_result {
                return Err(e.into());
            }
        }
        // If work failed, return the original work error even if lock release also failed.
        result
    }
}

impl AggregatorBatchingTrigger {
    async fn calculate_range(&self, config: &Arc<Config>) -> Result<(u64, u64), JobError> {
        // Get the latest aggregator batch from the DB
        let latest_batch = config.database().get_latest_aggregator_batch().await?;

        // Getting the latest block numbers for aggregator and snos batches from DB
        let last_processed_block = latest_batch.map_or(-1, |batch| batch.end_block as i64);

        // Getting the latest block number from the sequencer
        let provider = config.madara_rpc_client();
        let last_block_in_provider =
            provider.block_number().await.map_err(|e| JobError::ProviderError(e.to_string()))?;

        // Calculating the last block number that needs to be assigned to a batch
        let last_block = config
            .service_config()
            .max_block_to_process
            .map_or(last_block_in_provider, |max_block| min(max_block, last_block_in_provider));

        debug!(last_block = %last_block, "Calculated last block number to batch.");

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block = max(config.service_config().min_block_to_process, (last_processed_block + 1) as u64);

        debug!(first_block = %first_block, "Calculated first block number to batch.");

        let last_block = min(last_block, first_block + config.params.batching_config.max_batch_processing_size - 1);
        Ok((first_block, last_block))
    }
}
