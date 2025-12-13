use crate::core::client::lock::LockValue;
use crate::core::config::Config;
use crate::error::job::JobError;
use crate::worker::event_handler::triggers::batching_v2::snos::{
    SnosBatchLimits, SnosHandler, SnosState, SnosStateHandler,
};
use crate::worker::event_handler::triggers::batching_v2::BlockProcessingResult;
use crate::worker::event_handler::triggers::JobTrigger;
use orchestrator_utils::layer::Layer;
use starknet::providers::Provider;
use std::cmp::{max, min};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct SnosBatchingTrigger;

impl JobTrigger for SnosBatchingTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Trying to acquire lock on SNOS Batching Worker (Taking a lock for 1 hr)
        match config
            .lock()
            .acquire_lock(
                "SnosBatchingWorker",
                LockValue::Boolean(false),
                config.params.batching_config.batching_worker_lock_duration,
                None,
            )
            .await
        {
            Ok(_) => {
                // Lock acquired successfully
                debug!("SnosBatchingWorker acquired lock");
            }
            Err(err) => {
                // Failed to acquire lock
                // Returning safely
                debug!("SnosBatchingWorker failed to acquire lock, returning safely: {}", err);
                return Ok(());
            }
        }

        let batching_handler = SnosHandler::new(
            config.madara_feeder_gateway_client(),
            SnosBatchLimits::from_config(&config.params),
            config.params.batching_config.default_empty_block_proving_gas,
        );

        let state_handler = SnosStateHandler::from_config(&config);

        // Execute the main work and capture the result
        let result = async {
            // Check and close any existing batch if needed before processing
            // self.check_and_close_existing_batch(&config).await?;

            let (start_block, end_block) = match config.layer() {
                Layer::L2 => self.calculate_range_l2(&config).await?,
                Layer::L3 => self.calculate_range_l3(&config).await?,
            };

            // If there are no blocks to process, return early
            if start_block > end_block {
                debug!("No blocks to process: start_block ({}) > end_block ({})", start_block, end_block);
                return Ok(());
            }

            info!("Processing SNOS batches for blocks {} to {}", start_block, end_block);

            let mut state = state_handler.load_batch_state().await?;

            for block_num in start_block..=end_block {
                // For L2s, we need to get the aggregator batch index for this block
                // For L3s, aggregator_batch_index is None
                let aggregator_batch_index = match config.layer() {
                    Layer::L2 => {
                        // Get the aggregator batch for this block
                        match config.database().get_aggregator_batch_for_block(block_num).await? {
                            Some(batch) => Some(batch.index),
                            None => {
                                // This should not happen as we calculate the range based on aggregator batches
                                warn!(
                                    "Block {} does not have an aggregator batch assigned, stopping SNOS batching",
                                    block_num
                                );
                                break;
                            }
                        }
                    }
                    Layer::L3 => {
                        // For L3s, we don't have aggregator batches
                        None
                    }
                };

                match batching_handler.include_block(block_num, aggregator_batch_index, state).await? {
                    BlockProcessingResult::Accumulated(updated_state) => {
                        state = updated_state;
                    }
                    BlockProcessingResult::BatchCompleted { completed_state, new_state } => {
                        match completed_state {
                            SnosState::Empty(_) => {
                                warn!("SNOS state contains empty state");
                                break;
                            }
                            SnosState::NonEmpty(completed_state) => {
                                state_handler.save_batch_state(&completed_state).await?;
                            }
                        }
                        state = new_state;
                    }
                    BlockProcessingResult::NotBatched => {
                        match state {
                            SnosState::Empty(_) => {}
                            SnosState::NonEmpty(state) => {
                                state_handler.save_batch_state(&state).await?;
                            }
                        }
                        break;
                    }
                }
            }

            // Save the final state if it's non-empty
            // match state {
            //     SnosState::Empty(_) => {}
            //     SnosState::NonEmpty(state) => {
            //         state_handler.save_batch_state(&state).await?;
            //     }
            // }

            Ok(())
        }
        .await;

        // Always release the lock, regardless of whether work succeeded or failed
        if let Err(e) = config.lock().release_lock("SnosBatchingWorker", None).await {
            error!("Failed to release SnosBatchingWorker lock: {}", e);
            // If work succeeded but lock release failed, return the lock release error
            if result.is_ok() {
                return Err(e.into());
            }
            // If work failed, we still want to return the original work error
        }

        result
    }
}

impl SnosBatchingTrigger {
    /// Calculate the range of blocks to process for L2s
    ///
    /// For L2s, SNOS batching depends on aggregator batching:
    /// - Start: max(last block SNOS batching processed, min_block_to_process)
    /// - End: min(last block processed by aggregator batching, max_block_to_process, start + max_blocks_to_process_at_once - 1)
    async fn calculate_range_l2(&self, config: &Arc<Config>) -> Result<(u64, u64), JobError> {
        // Get the latest SNOS batch from the DB
        let latest_snos_batch = config.database().get_latest_snos_batch().await?;

        // Get the latest aggregator batch from the DB
        let latest_aggregator_batch = config.database().get_latest_aggregator_batch().await?;

        // Get the last block processed by SNOS batching
        let last_snos_block_in_db = latest_snos_batch.map_or(-1, |batch| batch.end_block as i64);

        // Get the last block processed by aggregator batching
        let last_aggregator_block_in_db = match latest_aggregator_batch {
            Some(batch) => batch.end_block as i64,
            None => {
                // No aggregator batch exists, so we can't process any SNOS batches
                debug!("No aggregator batch exists, cannot process SNOS batches for L2");
                return Ok((1, 0)); // Return invalid range to skip processing
            }
        };

        // Calculate the first block to assign to SNOS batch
        let first_block_to_assign_batch =
            max(config.service_config().min_block_to_process, (last_snos_block_in_db + 1) as u64);

        // Calculate the last block to assign to SNOS batch
        // For L2s, we can only process blocks that have been assigned to aggregator batches
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(last_aggregator_block_in_db as u64, |max_block| min(max_block, last_aggregator_block_in_db as u64));

        debug!(
            first_block_to_assign_batch = %first_block_to_assign_batch,
            last_block_to_assign_batch = %last_block_to_assign_batch,
            "Calculated SNOS batch range for L2"
        );

        // Cap the range by max_blocks_to_process_at_once
        let max_blocks_to_process_at_once = self.max_blocks_to_process_at_once();
        let end_block =
            min(last_block_to_assign_batch, first_block_to_assign_batch + max_blocks_to_process_at_once - 1);

        Ok((first_block_to_assign_batch, end_block))
    }

    /// Calculate the range of blocks to process for L3s
    ///
    /// For L3s, SNOS batching is independent:
    /// - Start: max(last block SNOS batching processed, min_block_to_process)
    /// - End: min(last block in provider, max_block_to_process, start + max_blocks_to_process_at_once - 1)
    async fn calculate_range_l3(&self, config: &Arc<Config>) -> Result<(u64, u64), JobError> {
        // Get the latest SNOS batch from the DB
        let latest_snos_batch = config.database().get_latest_snos_batch().await?;

        // Get the last block processed by SNOS batching
        let last_snos_block_in_db = latest_snos_batch.map_or(-1, |batch| batch.end_block as i64);

        // Get the latest block number from the sequencer
        let provider = config.madara_rpc_client();
        let block_number_provider =
            provider.block_number().await.map_err(|e| JobError::ProviderError(e.to_string()))?;

        // Calculate the first block to assign to SNOS batch
        let first_block_to_assign_batch =
            max(config.service_config().min_block_to_process, (last_snos_block_in_db + 1) as u64);

        // Calculate the last block to assign to SNOS batch
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(block_number_provider, |max_block| min(max_block, block_number_provider));

        debug!(
            first_block_to_assign_batch = %first_block_to_assign_batch,
            last_block_to_assign_batch = %last_block_to_assign_batch,
            "Calculated SNOS batch range for L3"
        );

        // Cap the range by max_blocks_to_process_at_once
        let max_blocks_to_process_at_once = self.max_blocks_to_process_at_once();
        let end_block =
            min(last_block_to_assign_batch, first_block_to_assign_batch + max_blocks_to_process_at_once - 1);

        Ok((first_block_to_assign_batch, end_block))
    }

    fn max_blocks_to_process_at_once(&self) -> u64 {
        25
    }
}
