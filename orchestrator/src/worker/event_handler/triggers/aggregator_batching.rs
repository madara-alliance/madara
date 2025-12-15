use crate::core::client::lock::LockValue;
use crate::core::config::Config;
use crate::error::job::JobError;
use crate::worker::event_handler::triggers::batching_v2::aggregator::{
    AggregatorBatchLimits, AggregatorHandler, AggregatorState, AggregatorStateHandler,
};
use crate::worker::event_handler::triggers::batching_v2::BlockProcessingResult;
use crate::worker::event_handler::triggers::JobTrigger;
use starknet::providers::Provider;
use std::cmp::{max, min};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct AggregatorBatchingTrigger;

#[async_trait::async_trait]
impl JobTrigger for AggregatorBatchingTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Trying to acquire lock on Aggregator Batching Worker
        match config
            .lock()
            .acquire_lock(
                "AggregatorBatchingWorker",
                LockValue::Boolean(false),
                config.params.batching_config.batching_worker_lock_duration,
                None,
            )
            .await
        {
            Ok(_) => {
                // Lock acquired successfully
                debug!("AggregatorBatchingWorker acquired lock");
            }
            Err(err) => {
                // Failed to acquire lock
                // Returning safely
                debug!("AggregatorBatchingWorker failed to acquire lock, returning safely: {}", err);
                return Ok(());
            }
        }

        let batching_handler = AggregatorHandler::new(
            config.clone(),
            AggregatorBatchLimits::from_config(&config.params),
            config.params.batching_config.default_empty_block_proving_gas,
        );

        let state_handler = AggregatorStateHandler::from_config(&config);

        // Execute the main work and capture the result
        let result = async {
            let (start_block, end_block) = self.calculate_range(&config).await?;

            // If there are no blocks to process, return early
            if start_block > end_block {
                debug!("No Aggregator blocks to process: start_block ({}) > end_block ({})", start_block, end_block);
                return Ok(());
            }

            info!("Processing Aggregator batches for blocks {} to {}", start_block, end_block);

            let mut state = state_handler.load_batch_state().await?;

            for block_num in start_block..=end_block {
                match batching_handler.include_block(block_num, state).await? {
                    BlockProcessingResult::Accumulated(updated_state) => {
                        state = updated_state;
                    }
                    BlockProcessingResult::BatchCompleted { completed_state, new_state } => {
                        match completed_state {
                            AggregatorState::Empty(_) => {
                                warn!("Aggregator state contains empty state");
                                state = completed_state;
                                break;
                            }
                            AggregatorState::NonEmpty(completed_state) => {
                                state_handler.save_batch_state(&completed_state).await?;
                            }
                        }
                        state = new_state;
                    }
                    BlockProcessingResult::NotBatched(current_state) => {
                        state = current_state;
                        break;
                    }
                }
            }

            match state {
                AggregatorState::Empty(_) => {}
                AggregatorState::NonEmpty(state) => {
                    state_handler.save_batch_state(&state).await?;
                }
            }

            Ok(())
        }
        .await;

        // Always release the lock, regardless of whether work succeeded or failed
        if let Err(e) = config.lock().release_lock("AggregatorBatchingWorker", None).await {
            error!("Failed to release AggregatorBatchingWorker lock: {}", e);
            // If work succeeded but lock release failed, return the lock release error
            if result.is_ok() {
                return Err(e.into());
            }
            // If work failed, we still want to return the original work error
        }

        result
    }
}

impl AggregatorBatchingTrigger {
    async fn calculate_range(&self, config: &Arc<Config>) -> Result<(u64, u64), JobError> {
        // Get the latest aggregator batch from the DB
        let latest_batch = config.database().get_latest_aggregator_batch().await?;

        // Getting the latest block numbers for aggregator and snos batches from DB
        let last_block_in_batches = latest_batch.map_or(-1, |batch| batch.end_block as i64);

        // Getting the latest block number from the sequencer
        let provider = config.madara_rpc_client();
        let last_block_in_provider =
            provider.block_number().await.map_err(|e| JobError::ProviderError(e.to_string()))?;

        // Calculating the last block number that needs to be assigned to a batch
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(last_block_in_provider, |max_block| min(max_block, last_block_in_provider));

        debug!(latest_block_number = %last_block_to_assign_batch, "Calculated last block number to batch.");

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block_to_assign_batch =
            max(config.service_config().min_block_to_process, (last_block_in_batches + 1) as u64);

        debug!(first_block_to_assign_batch = %first_block_to_assign_batch, "Calculated first block number to batch.");

        let max_blocks_to_process_at_once = self.max_blocks_to_process_at_once();
        let end_block =
            min(last_block_to_assign_batch, first_block_to_assign_batch + max_blocks_to_process_at_once - 1);
        Ok((first_block_to_assign_batch, end_block))
    }

    fn max_blocks_to_process_at_once(&self) -> u64 {
        10
    }
}
