use crate::compression::blob::{convert_felt_vec_to_blob_data, state_update_to_blob_data};
use crate::compression::squash::squash;
use crate::compression::stateful::compress as stateful_compress;
use crate::compression::stateless::compress as stateless_compress;
use crate::core::client::lock::LockValue;
use crate::core::config::{Config, StarknetVersion};
use crate::core::StorageClient;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, SnosBatch, SnosBatchStatus, SnosBatchUpdates,
};
use crate::types::constant::{STORAGE_BLOB_DIR, STORAGE_STATE_UPDATE_DIR};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::triggers::snos::fetch_block_starknet_version;
use crate::worker::event_handler::triggers::JobTrigger;
use crate::worker::utils::biguint_vec_to_u8_vec;
use blockifier::bouncer::BouncerWeights;
use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use opentelemetry::KeyValue;
use orchestrator_prover_client_interface::Task;
use starknet::core::types::{BlockId, StateUpdate};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet_core::types::Felt;
use starknet_core::types::MaybePreConfirmedStateUpdate::{PreConfirmedUpdate, Update};
use std::cmp::{max, min};
use std::sync::Arc;
use std::time::Instant;
use tokio::try_join;
use tracing::{debug, error, info, trace, warn};

pub struct BatchingTrigger;

struct BatchState<'a> {
    aggregator_batch: &'a AggregatorBatch,
    snos_batch: &'a SnosBatch,
    close_aggregator_batch: bool,       // boolean to decide if we can close the block
    snos_batch_status: SnosBatchStatus, // New status of SNOS batch
    state_update: &'a StateUpdate,
}

// Community doc for v0.13.2 - https://community.starknet.io/t/starknet-v0-13-2-pre-release-notes/114223

#[async_trait::async_trait]
impl JobTrigger for BatchingTrigger {
    /// 1. Fetch the latest completed block from Starknet chain
    /// 2. Fetch the last batch and check its `end_block`
    /// 3. Assign batches to all the remaining blocks and store the squashed state update in storage
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        info!(log_type = "starting", "BatchingWorker started");

        // Trying to acquire lock on Batching Worker (Taking a lock for 1 hr)
        match config
            .lock()
            .acquire_lock(
                "BatchingWorker",
                LockValue::Boolean(false),
                config.params.batching_config.batching_worker_lock_duration,
                None,
            )
            .await
        {
            Ok(_) => {
                // Lock acquired successfully
                info!("BatchingWorker acquired lock");
            }
            Err(err) => {
                // Failed to acquire lock
                // Returning safely
                info!("BatchingWorker failed to acquire lock, returning safely: {}", err);
                return Ok(());
            }
        }

        // Getting the latest aggregator and snos batch in DB
        let latest_aggregator_batch = config.database().get_latest_aggregator_batch().await?;
        let latest_snos_batch = config.database().get_latest_snos_batch().await?;

        // Check if any existing batch needs to be closed
        if let Some(aggregator_batch) = &latest_aggregator_batch {
            if let Some(snos_batch) = &latest_snos_batch {
                self.check_and_close_batches(&config, aggregator_batch, snos_batch).await?;
            } else {
                return Err(eyre!("Aggregator and SNOS batches are out of sync. We have an Aggregator batch ({}) in the DB but no SNOS batch", aggregator_batch.index));
            }
        }

        // Getting the latest block numbers for aggregator and snos batches from DB1
        let latest_aggregator_block_in_db = latest_aggregator_batch.map_or(-1, |batch| batch.end_block as i64);
        let latest_snos_block_in_db = latest_snos_batch.map_or(-1, |batch| batch.end_block as i64);

        // Ensure aggregator and SNOS batches are in sync
        if latest_aggregator_block_in_db != latest_snos_block_in_db {
            return Err(eyre!(
                "Aggregator and SNOS batches are out of sync: aggregator_block={}, snos_block={}",
                latest_aggregator_block_in_db,
                latest_snos_block_in_db
            ));
        }

        // Getting the latest block number from the sequencer
        let provider = config.madara_client();
        let block_number_provider = provider.block_number().await?;

        // Calculating the last block number that needs to be assigned to a batch
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(block_number_provider, |max_block| min(max_block, block_number_provider));

        debug!(latest_block_number = %last_block_to_assign_batch, "Calculated last block number to batch.");

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block_to_assign_batch =
            max(config.service_config().min_block_to_process, (latest_aggregator_block_in_db + 1) as u64);

        debug!(first_block_to_assign_batch = %first_block_to_assign_batch, "Calculated first block number to batch.");

        if first_block_to_assign_batch <= last_block_to_assign_batch {
            let (start_block, end_block) =
                self.get_blocks_range_to_process(first_block_to_assign_batch, last_block_to_assign_batch);
            self.assign_batch_to_blocks(start_block, end_block, config.clone()).await?;
        }

        // Releasing the lock
        config.lock().release_lock("BatchingWorker", None).await?;

        trace!(log_type = "completed", "BatchingWorker completed.");
        Ok(())
    }
}

impl BatchingTrigger {
    /// assign_batch_to_blocks assigns a batch to all the blocks from `start_block_number` to
    /// `end_block_number` and updates the state in DB and stores the output in storage
    /// This done for both kind of blocks, aggregator and SNOS
    async fn assign_batch_to_blocks(
        &self,
        start_block_number: u64,
        end_block_number: u64,
        config: Arc<Config>,
    ) -> Result<(), JobError> {
        if end_block_number < start_block_number {
            return Err(JobError::Other(OtherError(eyre!(
                "Failed to assign batch to blocks as end_block_number ({}) is smaller than start_block_number ({})",
                end_block_number,
                start_block_number
            ))));
        }

        info!("Assigning batches to blocks from {} to {}", start_block_number, end_block_number);

        tracing::Span::current().record("block_start", start_block_number);
        tracing::Span::current().record("block_end", end_block_number);
        // Get the database
        let database = config.database();

        // Get the storage client
        let storage = config.storage();

        // Get the latest snos batch from the database
        let latest_snos_batch_option = config.database().get_latest_snos_batch().await?;

        // Get the latest batches and state update
        let (mut latest_aggregator_batch, mut latest_snos_batch, mut state_update) = match database
            .get_latest_aggregator_batch()
            .await?
        {
            Some(aggregator_batch) => {
                // The latest batch is full. Start a new batch
                // We close the SNOS batch as well when the aggregator batch is closed, so don't need to do anything here
                // We are assuming that the SNOS batch is closed here

                let snos_batch = latest_snos_batch_option.ok_or(JobError::BatchingNotInSync(format!(
                    "No SNOS batch present in DB but Aggregator batch ({}) is present",
                    aggregator_batch.index
                )))?;

                // Check if there is a status conflict between the latest snos and aggregator batch
                if (aggregator_batch.is_batch_ready && snos_batch.status != SnosBatchStatus::Closed)
                    || (!aggregator_batch.is_batch_ready && snos_batch.status == SnosBatchStatus::Closed)
                {
                    return Err(JobError::BatchingNotInSync(format!(
                        "Latest SNOS batch {} is {} but Latest Aggregator batch {} is {}",
                        snos_batch.snos_batch_id, snos_batch.status, aggregator_batch.index, aggregator_batch.status
                    )));
                }

                // Check if there is an end block conflict between the latest snos and aggregator batch
                if snos_batch.end_block != aggregator_batch.end_block {
                    return Err(JobError::BatchingNotInSync(format!(
                        "Latest SNOS batch {}'s end block is {} but latest Aggregator batch {}'s end block is {}",
                        snos_batch.snos_batch_id,
                        snos_batch.end_block,
                        aggregator_batch.index,
                        aggregator_batch.end_block
                    )));
                }

                if aggregator_batch.is_batch_ready {
                    // Both batches are full. Create new batches
                    let (new_snos_batch, new_aggregator_batch) = self
                        .start_new_batches(
                            &config,
                            aggregator_batch.index + 1,
                            snos_batch.snos_batch_id + 1,
                            start_block_number,
                        )
                        .await?;
                    (new_aggregator_batch, new_snos_batch, None)
                } else {
                    // Previous batch is not full, continue with the previous batch
                    let state_update_bytes = storage.get_data(&aggregator_batch.squashed_state_updates_path).await?;
                    let state_update: StateUpdate = serde_json::from_slice(&state_update_bytes)?;
                    (aggregator_batch, snos_batch, Some(state_update))
                }
            }
            None => {
                match latest_snos_batch_option {
                    Some(snos_batch) => {
                        return Err(JobError::BatchingNotInSync(format!(
                            "No Aggregator batch present in DB but SNOS batch ({}) is present",
                            snos_batch.snos_batch_id
                        )))
                    }
                    None => {
                        // No batch in DB. Start a new batch
                        let (new_snos_batch, new_aggregator_batch) =
                            self.start_new_batches(&config, 1, 1, start_block_number).await?;
                        (new_aggregator_batch, new_snos_batch, None)
                    }
                }
            }
        };

        // Assign batches to all the blocks
        for block_number in start_block_number..=end_block_number {
            (state_update, latest_aggregator_batch, latest_snos_batch) = self
                .assign_batch(block_number, state_update, latest_aggregator_batch, latest_snos_batch, &config)
                .await?;
            tracing::Span::current().record("batch_id", latest_aggregator_batch.index);
        }

        // This just updates the aggregator/snos batch in the DB and does not close the batch
        if let Some(state_update) = state_update {
            self.save_batch_state(
                BatchState {
                    aggregator_batch: &latest_aggregator_batch,
                    snos_batch: &latest_snos_batch,
                    close_aggregator_batch: false, // Don't close the aggregator batch
                    snos_batch_status: SnosBatchStatus::Open, // Open the SNOS batch
                    state_update: &state_update,
                },
                &config,
                config.madara_client(),
            )
            .await?;
        }
        Ok(())
    }

    /// assign_batch assigns a batch to a block
    /// takes the squashed state update till now, and the current batch
    /// returns the new state update, and the batch
    /// this function assumes that the `current_batch` is not ready
    async fn assign_batch(
        &self,
        block_number: u64,
        prev_state_update: Option<StateUpdate>,
        current_aggregator_batch: AggregatorBatch,
        current_snos_batch: SnosBatch,
        config: &Arc<Config>,
    ) -> Result<(Option<StateUpdate>, AggregatorBatch, SnosBatch), JobError> {
        // Get the provider
        let provider = config.madara_client();

        // Fetch Starknet version for the current block
        let current_block_starknet_version = fetch_block_starknet_version(config, block_number).await.map_err(|e| {
            JobError::ProviderError(format!("Failed to fetch Starknet version for block {}: {}", block_number, e))
        })?;

        // Check if current block's Starknet version differs from the aggregator batch version
        // A batch can only contain blocks from the same Starknet protocol version (prover requirement)
        // Since SNOS batches belong to aggregator batches, they automatically inherit version consistency
        if current_aggregator_batch.starknet_version != current_block_starknet_version {
            info!(
                block_number = %block_number,
                current_block_starknet_version = %current_block_starknet_version,
                batch_starknet_version = %current_aggregator_batch.starknet_version,
                "Starknet version mismatch detected, closing current batches to maintain version consistency across the bucket"
            );

            // Close both current batches if there's a previous state update
            if let Some(ref prev_update) = prev_state_update {
                self.save_batch_state(
                    BatchState {
                        aggregator_batch: &current_aggregator_batch,
                        snos_batch: &current_snos_batch,
                        close_aggregator_batch: true,
                        snos_batch_status: SnosBatchStatus::Closed,
                        state_update: prev_update,
                    },
                    config,
                    provider,
                )
                .await?;
            }

            // Start new batches with the current block's Starknet version
            let (new_snos_batch, new_aggregator_batch) = self
                .start_new_batches(
                    config,
                    current_aggregator_batch.index + 1,
                    current_snos_batch.snos_batch_id + 1,
                    block_number,
                )
                .await?;

            // Get state update for the current block
            let current_state_update = provider
                .get_state_update(BlockId::Number(block_number))
                .await
                .map_err(|e| JobError::ProviderError(e.to_string()))?;

            return match current_state_update {
                Update(state_update) => Ok((Some(state_update), new_aggregator_batch, new_snos_batch)),
                PreConfirmedUpdate(_) => {
                    info!("Skipping batching for block {} as it is still pending", block_number);
                    Ok((None, new_aggregator_batch, new_snos_batch))
                }
            };
        }

        // Get the state update for the block
        let current_state_update = provider
            .get_state_update(BlockId::Number(block_number))
            .await
            .map_err(|e| JobError::ProviderError(e.to_string()))?;

        match current_state_update {
            Update(state_update) => {
                match prev_state_update {
                    Some(prev_state_update) => {
                        // Squash the state updates
                        let squashed_state_update = squash(
                            vec![&prev_state_update, &state_update],
                            if current_aggregator_batch.start_block == 0 {
                                None
                            } else {
                                Some(current_aggregator_batch.start_block - 1)
                            },
                            provider,
                        )
                        .await?;
                        // Compress the squashed state update based on the madara version
                        let compressed_state_update = self
                            .compress_state_update(
                                &squashed_state_update,
                                config.params.madara_version,
                                block_number.saturating_sub(1),
                                provider,
                            )
                            .await?;
                        if self.should_close_aggregator_batch(
                            config,
                            Some(compressed_state_update.len()),
                            &current_aggregator_batch,
                        ) {
                            // We cannot add the current block in this batch

                            // Close the current batches (both aggregator and SNOS) and save the state
                            self.save_batch_state(
                                BatchState {
                                    aggregator_batch: &current_aggregator_batch,
                                    snos_batch: &current_snos_batch,
                                    close_aggregator_batch: true, // Close the aggregator batch
                                    snos_batch_status: SnosBatchStatus::Closed, // Close the SNOS batch
                                    state_update: &squashed_state_update,
                                },
                                config,
                                provider,
                            )
                            .await?;

                            // Start a new Aggregator batch
                            let (new_snos_batch, new_aggregator_batch) = self
                                .start_new_batches(
                                    config,
                                    current_aggregator_batch.index + 1,
                                    current_snos_batch.snos_batch_id + 1,
                                    block_number,
                                )
                                .await?;
                            Ok((Some(state_update), new_aggregator_batch, new_snos_batch))
                        } else if self.should_close_snos_batch(config, &current_snos_batch).await? {
                            // Close the current SNOS batch and start a new one
                            self.save_batch_state(
                                BatchState {
                                    aggregator_batch: &current_aggregator_batch,
                                    snos_batch: &current_snos_batch,
                                    close_aggregator_batch: false, // Don't close the aggregator batch
                                    snos_batch_status: SnosBatchStatus::Closed, // Close the SNOS batch
                                    state_update: &squashed_state_update,
                                },
                                config,
                                provider,
                            )
                            .await?;

                            // Starting a new SNOS batch
                            let new_snos_batch = self.start_snos_batch(
                                current_snos_batch.snos_batch_id + 1,
                                current_aggregator_batch.index,
                                block_number,
                            )?;

                            Ok((
                                Some(state_update),
                                self.update_aggregator_batch_info(
                                    current_aggregator_batch,
                                    block_number,
                                    Some(new_snos_batch.snos_batch_id),
                                    false,
                                )
                                .await?,
                                new_snos_batch,
                            ))
                        } else {
                            // We can add the current block in this batch
                            // Update batch info and return
                            Ok((
                                Some(squashed_state_update),
                                self.update_aggregator_batch_info(current_aggregator_batch, block_number, None, false)
                                    .await?,
                                self.update_snos_batch_info(current_snos_batch, block_number).await?,
                            ))
                        }
                    }
                    None => Ok((
                        Some(state_update),
                        self.update_aggregator_batch_info(current_aggregator_batch, block_number, None, false).await?,
                        self.update_snos_batch_info(current_snos_batch, block_number).await?,
                    )),
                }
            }
            PreConfirmedUpdate(_) => {
                info!("Skipping batching for block {} as it is still pending", block_number);
                Ok((prev_state_update, current_aggregator_batch, current_snos_batch))
            }
        }
    }

    /// Function to create a new Aggregator batch.
    /// Sends an API request to the prover client to create a new bucket.
    /// Returns the new Aggregator batch.
    ///
    /// NOTE that it does not put anything in the DB. It just creates a new struct and returns it.
    async fn start_aggregator_batch(
        &self,
        config: &Arc<Config>,
        index: u64,
        start_snos_batch: u64,
        start_block: u64,
    ) -> Result<AggregatorBatch, JobError> {
        // Start timing batch creation
        let start_time = Instant::now();

        // Fetch Starknet version for the start block
        // In tests, use a default version if fetch fails due to HTTP mocking limitations
        let starknet_version = fetch_block_starknet_version(config, start_block).await.map_err(|e| {
            error!(bucket_index = %index, error = %e, "Failed to submit create bucket task to prover client, {}", e);
            JobError::Other(OtherError(eyre!("Failed to fetch Starknet version for block {}: {}", start_block, e)))
        })?;
        info!(
            index = %index,
            start_block = %start_block,
            starknet_version = %starknet_version,
            "Fetched Starknet version for new batch"
        );

        // Start a new bucket
        let bucket_id = config.prover_client().submit_task(Task::CreateBucket).await.map_err(|e| {
            error!(bucket_index = %index, error = %e, "Failed to submit create bucket task to prover client, {}", e);
            JobError::Other(OtherError(eyre!(
                "Prover Client Error: Failed to submit create bucket task to prover client, {}",
                e
            ))) // TODO: Add a new error type to be used for prover client error
        })?;
        info!(index = %index, bucket_id = %bucket_id, "Created new bucket successfully");

        let batch = AggregatorBatch::new(
            index,
            start_snos_batch,
            start_block,
            self.get_state_update_file_path(index),
            self.get_blob_dir_path(index),
            bucket_id.clone(),
            starknet_version.clone(),
        );

        // Record batch creation time with starknet_version in metrics
        let duration = start_time.elapsed();
        let attributes = [
            KeyValue::new("batch_index", index.to_string()),
            KeyValue::new("start_block", start_block.to_string()),
            KeyValue::new("bucket_id", bucket_id.to_string()),
            KeyValue::new("starknet_version", starknet_version),
        ];
        ORCHESTRATOR_METRICS.batch_creation_time.record(duration.as_secs_f64(), &attributes);

        // Update batching rate (batches per hour)
        // This is a simple counter that will be used to calculate rate in Grafana
        ORCHESTRATOR_METRICS.batching_rate.record(1.0, &attributes);

        info!(
            index = %index,
            duration_seconds = %duration.as_secs_f64(),
            "Batch created successfully"
        );

        Ok(batch)
    }

    /// Function to create a new SNOS batch.
    ///
    /// NOTE that it does not put anything in the DB. It just creates a new struct and returns it.
    fn start_snos_batch(
        &self,
        snos_batch_id: u64,
        aggregator_batch_index: u64,
        start_block: u64,
    ) -> Result<SnosBatch, JobError> {
        Ok(SnosBatch::new(snos_batch_id, aggregator_batch_index, start_block))
    }

    /// Creates and returns new SNOS and Aggregator batches
    async fn start_new_batches(
        &self,
        config: &Arc<Config>,
        aggregator_batch_index: u64,
        snos_index: u64,
        start_block: u64,
    ) -> Result<(SnosBatch, AggregatorBatch), JobError> {
        let snos_batch = self.start_snos_batch(snos_index, aggregator_batch_index, start_block)?;
        let aggregator_batch =
            self.start_aggregator_batch(config, aggregator_batch_index, snos_index, start_block).await?;
        Ok((snos_batch, aggregator_batch))
    }

    /// Updates the aggregator batch status in the database
    async fn update_or_close_aggregator_batch(
        &self,
        aggregator_batch: &AggregatorBatch,
        close_aggregator_batch: bool, // boolean to decide if we can close the block
        config: &Arc<Config>,
    ) -> Result<(), JobError> {
        // Get the database
        let database = config.database();

        // Update batch status in the database
        database
            .update_or_create_aggregator_batch(
                aggregator_batch,
                &AggregatorBatchUpdates {
                    end_snos_batch: Some(aggregator_batch.end_snos_batch),
                    end_block: Some(aggregator_batch.end_block),
                    is_batch_ready: Some(close_aggregator_batch),
                    status: if close_aggregator_batch { Some(AggregatorBatchStatus::Closed) } else { None },
                },
            )
            .await?;

        Ok(())
    }

    /// Stores the state update and blob info in storage
    async fn store_aggregator_batch_state_update(
        &self,
        aggregator_batch: &AggregatorBatch,
        state_update: &StateUpdate,
        config: &Arc<Config>,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<(), JobError> {
        let storage = config.storage();

        let compressed_state_update = self
            .compress_state_update(state_update, config.params.madara_version, aggregator_batch.end_block, provider)
            .await?;
        try_join!(
            // Update state update and blob for the batch in storage
            self.store_state_update(storage, state_update, aggregator_batch),
            self.store_blob(storage, &compressed_state_update, aggregator_batch),
        )?;

        Ok(())
    }

    async fn update_or_close_snos_batch(
        &self,
        snos_batch: &SnosBatch,
        config: &Arc<Config>,
        status: SnosBatchStatus,
    ) -> Result<(), JobError> {
        let database = config.database();

        database
            .update_or_create_snos_batch(
                snos_batch,
                &SnosBatchUpdates { end_block: Some(snos_batch.end_block), status: Some(status) },
            )
            .await?;

        Ok(())
    }

    /// Saves the current state of the whole batching system in the DB and storage.
    ///
    /// Does the following:
    /// 1. Store the state update and blob info in storage
    /// 2. Update or add the state of the Aggregator batch in DB
    /// 3. Update or add the state of an SNOS batch in the DB
    async fn save_batch_state<'a>(
        &self,
        batch_state: BatchState<'a>,
        config: &Arc<Config>,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<(), JobError> {
        try_join!(
            self.store_aggregator_batch_state_update(
                batch_state.aggregator_batch,
                batch_state.state_update,
                config,
                provider
            ),
            self.update_or_close_aggregator_batch(
                batch_state.aggregator_batch,
                batch_state.close_aggregator_batch,
                config,
            ),
            self.update_or_close_snos_batch(batch_state.snos_batch, config, batch_state.snos_batch_status),
        )?;

        Ok(())
    }

    /// Updates the Aggregator batch information in the mut AggregatorBatch argument
    async fn update_aggregator_batch_info(
        &self,
        mut batch: AggregatorBatch,
        end_block: u64,
        end_snos_batch: Option<u64>,
        is_batch_ready: bool,
    ) -> Result<AggregatorBatch, JobError> {
        batch.end_block = end_block;
        if let Some(end_snos_batch) = end_snos_batch {
            batch.end_snos_batch = end_snos_batch;
        }
        batch.is_batch_ready = is_batch_ready;
        batch.num_blocks = end_block - batch.start_block + 1;
        Ok(batch)
    }

    /// Updates the SNOS batch information in the mut SnosBatch argument
    async fn update_snos_batch_info(&self, mut batch: SnosBatch, end_block: u64) -> Result<SnosBatch, JobError> {
        batch.end_block = end_block;
        batch.num_blocks = end_block - batch.start_block + 1;
        Ok(batch)
    }

    async fn compress_state_update(
        &self,
        state_update: &StateUpdate,
        madara_version: StarknetVersion,
        end_block: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<Vec<Felt>, JobError> {
        // Perform stateful compression
        let state_update = if madara_version >= StarknetVersion::V0_13_4 {
            stateful_compress(state_update, end_block, provider)
                .await
                .map_err(|err| JobError::Other(OtherError(err)))?
        } else {
            state_update.clone()
        };
        // Get a vector of felts from the compressed state update
        let vec_felts = state_update_to_blob_data(state_update, madara_version).await?;
        // Perform stateless compression
        if madara_version >= StarknetVersion::V0_13_3 {
            stateless_compress(&vec_felts).map_err(|err| JobError::Other(OtherError(err)))
        } else {
            Ok(vec_felts)
        }
    }

    /// get_state_update_file_name returns the file path for storing the state update in storage
    fn get_state_update_file_path(&self, batch_index: u64) -> String {
        format!("{}/batch/{}.json", STORAGE_STATE_UPDATE_DIR, batch_index)
    }

    fn get_blob_file_path(&self, batch_index: u64, blob_index: u64) -> String {
        format!("{}/batch/{}/{}.txt", STORAGE_BLOB_DIR, batch_index, blob_index)
    }

    fn get_blob_dir_path(&self, batch_index: u64) -> String {
        format!("{}/batch/{}", STORAGE_BLOB_DIR, batch_index)
    }

    /// store_state_update stores the state_update in the DB
    async fn store_state_update(
        &self,
        storage: &dyn StorageClient,
        state_update: &StateUpdate,
        batch: &AggregatorBatch,
    ) -> Result<(), JobError> {
        storage
            .put_data(Bytes::from(serde_json::to_string(&state_update)?), &self.get_state_update_file_path(batch.index))
            .await?;
        Ok(())
    }

    /// store_blob stores the compressed_state_update in a blob format in the storage
    /// NOTE: compressed_state_update should be a vector of felts from which a blob is created
    /// Make sure that this is compressed according to the specs of the Starknet Version being used
    async fn store_blob(
        &self,
        storage: &dyn StorageClient,
        compressed_state_update: &[Felt],
        batch: &AggregatorBatch,
    ) -> Result<(), JobError> {
        let blobs = convert_felt_vec_to_blob_data(compressed_state_update)?;
        for (index, blob) in blobs.iter().enumerate() {
            storage
                .put_data(
                    biguint_vec_to_u8_vec(blob.as_slice()).into(),
                    &self.get_blob_file_path(batch.index, index as u64 + 1),
                )
                .await?;
        }
        Ok(())
    }

    fn get_blocks_range_to_process(&self, start_block: u64, end_block: u64) -> (u64, u64) {
        let max_blocks_to_process_at_once = self.max_blocks_to_process_at_once();
        let end_block = min(end_block, start_block + max_blocks_to_process_at_once - 1);
        (start_block, end_block)
    }

    fn max_blocks_to_process_at_once(&self) -> u64 {
        25 // TODO: should be taken from the config?
    }

    /// Determines whether a new batch should be started based on the size of the compressed
    /// state-update and the batch.
    /// Returns true if a new batch should be started, false otherwise.
    ///
    /// Starts a new batch if:
    /// 1. Length of the compressed state update has reached its max level
    /// 2. The number of blocks in the batch has reached max level
    /// 3. Time between now and when the batch started has exceeded max limit
    /// 4. Batch is not yet closed
    fn should_close_aggregator_batch(
        &self,
        config: &Arc<Config>,
        state_update_len: Option<usize>,
        batch: &AggregatorBatch,
    ) -> bool {
        debug!("checking if we need to close the aggregator batch");
        (!batch.is_batch_ready)
            && ((state_update_len.is_some() && state_update_len.unwrap() > config.params.batching_config.max_blob_size)
                || (batch.num_blocks >= config.params.batching_config.max_batch_size)
                || ((Utc::now().round_subsecs(0) - batch.created_at).abs().num_seconds() as u64
                    >= config.params.batching_config.max_batch_time_seconds))
    }

    async fn should_close_snos_batch(&self, config: &Arc<Config>, batch: &SnosBatch) -> Result<bool, JobError> {
        debug!("checking if we need to close the snos batch");
        if let Some(max_blocks_per_snos_batch) = config.params.batching_config.max_blocks_per_snos_batch {
            // If the MADARA_ORCHESTRATOR_MAX_BLOCKS_PER_SNOS_BATCH env is set, we use that value
            // Mostly, it'll be used for testing purposes
            warn!("Using MADARA_ORCHESTRATOR_MAX_BLOCKS_PER_SNOS_BATCH env variable to close snos batch with max blocks = {}", max_blocks_per_snos_batch);
            return Ok(batch.num_blocks >= max_blocks_per_snos_batch);
        }
        let current_builtins = BouncerWeights::empty();
        for block_number in batch.start_block..=batch.end_block {
            // Get the bouncer weights for this block
            info!("getting the bouncer weights for block number: {:?}", block_number);
            let bouncer_weights = self
                .get_block_builtin_weights(config, block_number)
                .await
                .map_err(|e| JobError::ProviderError(e.to_string()))?;
            current_builtins.checked_add(bouncer_weights);
        }
        Ok(config.params.bouncer_weights_limit.checked_sub(current_builtins).is_none())
    }

    /// Checks if we need to close the batches and closes them if needed.
    /// For now, it checks only if the aggregator batch needs to be closed, and if it does, we close
    /// both the batches (Aggregator and SNOS).
    /// This is intended to be used before the batching even begins so that we can close the batches
    /// on the batch time and length basis
    async fn check_and_close_batches(
        &self,
        config: &Arc<Config>,
        aggregator_batch: &AggregatorBatch,
        snos_batch: &SnosBatch,
    ) -> Result<(), JobError> {
        // Sending None for state update len since this won't be the reason to close an already existing batch
        if self.should_close_aggregator_batch(config, None, aggregator_batch) {
            config
                .database()
                .update_or_create_aggregator_batch(
                    aggregator_batch,
                    &AggregatorBatchUpdates {
                        end_snos_batch: Some(snos_batch.snos_batch_id),
                        end_block: Some(aggregator_batch.end_block),
                        is_batch_ready: Some(true),
                        status: Some(AggregatorBatchStatus::Closed),
                    },
                )
                .await?;

            config
                .database()
                .update_or_create_snos_batch(
                    snos_batch,
                    &SnosBatchUpdates { end_block: Some(snos_batch.end_block), status: Some(SnosBatchStatus::Closed) },
                )
                .await?;
        }

        Ok(())
    }

    /// Get the block builtin weights from Madara admin RPC
    /// This uses a custom admin method that's not part of standard Starknet RPC
    async fn get_block_builtin_weights(
        &self,
        config: &Arc<Config>,
        block_number: u64,
    ) -> Result<BouncerWeights, JobError> {
        use serde::{Deserialize, Serialize};

        #[derive(Serialize)]
        struct JsonRpcRequest {
            jsonrpc: String,
            method: String,
            params: AdminRequestData,
            id: u32,
        }

        #[derive(Serialize)]
        struct AdminRequestData {
            block_number: u64,
        }

        #[derive(Deserialize)]
        struct JsonRpcResponse {
            result: Option<BouncerWeights>,
            error: Option<serde_json::Value>,
        }

        // Create the JSON-RPC request in the style of starknet_providers
        let raw_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "madara_V0_1_0_getBlockBuiltinWeights".to_string(),
            params: AdminRequestData { block_number },
            id: 1,
        };

        // Serialize to JSON string to match the pattern you showed
        let request_body = serde_json::to_string(&raw_request)
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to serialize request: {}", e))))?;

        debug!(
            block_number = %block_number,
            request = %request_body,
            "Sending admin RPC request for block builtin weights"
        );

        // Make the HTTP request
        let client = reqwest::Client::new();
        let response = client
            .post(config.params.madara_admin_rpc_url.as_str())
            .header("Content-Type", "application/json")
            .body(request_body)
            .send()
            .await
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to send admin RPC request: {}", e))))?;

        // Parse the response
        let response_text = response
            .text()
            .await
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to read admin RPC response: {}", e))))?;

        let parsed_response: JsonRpcResponse = serde_json::from_str(&response_text)
            .map_err(|e| JobError::Other(OtherError(eyre!("Failed to parse admin RPC response: {}", e))))?;

        // Handle the response similar to starknet_providers pattern
        if let Some(error) = parsed_response.error {
            return Err(JobError::Other(OtherError(eyre!("Admin RPC error: {}", error))));
        }

        parsed_response.result.ok_or_else(|| JobError::Other(OtherError(eyre!("Missing result in admin RPC response"))))
    }
}
