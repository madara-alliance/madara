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
use crate::types::constant::{MAX_BLOB_SIZE, STORAGE_BLOB_DIR, STORAGE_STATE_UPDATE_DIR};
use crate::worker::event_handler::triggers::JobTrigger;
use crate::worker::utils::biguint_vec_to_u8_vec;
use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use orchestrator_prover_client_interface::Task;
use starknet::core::types::{BlockId, StateUpdate};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet_core::types::Felt;
use starknet_core::types::MaybePendingStateUpdate::{PendingUpdate, Update};
use std::cmp::{max, min};
use std::sync::Arc;
use tokio::try_join;

pub struct BatchingTrigger;

// Community doc for v0.13.2 - https://community.starknet.io/t/starknet-v0-13-2-pre-release-notes/114223

#[async_trait::async_trait]
impl JobTrigger for BatchingTrigger {
    /// 1. Fetch the latest completed block from Starknet chain
    /// 2. Fetch the last batch and check its `end_block`
    /// 3. Assign batches to all the remaining blocks and store the squashed state update in storage
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::info!(log_type = "starting", category = "BatchingWorker", "BatchingWorker started");

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
                tracing::info!("BatchingWorker acquired lock");
            }
            Err(err) => {
                // Failed to acquire lock
                // Returning safely
                tracing::info!("BatchingWorker failed to acquire lock, returning safely: {}", err);
                return Ok(());
            }
        }

        // Getting the latest batch in DB
        let latest_aggregator_batch = config.database().get_latest_aggregator_batch().await?;
        let latest_aggregator_block_in_db = latest_aggregator_batch.clone().map_or(-1, |batch| batch.end_block as i64);
        
        let mut latest_snos_batch = config.database().get_latest_snos_batch().await?;
        
        // Check if any existing batch needs to be closed
        if let Some(batch) = latest_aggregator_batch {
            if let Some(snos_batch) = latest_snos_batch.clone() {
                self.check_and_close_batches(&config, &batch, &snos_batch).await?;
            } else {
                // Create a new SNOS batch with the same index and block range as the aggregator batch
                // NOTE: Using index 1 for the SNOS batch since it is the first batch
                let snos_batch = SnosBatch::new(1, batch.start_block, batch.end_block);
                config.database().create_snos_batch(snos_batch.clone()).await?;
                latest_snos_batch = Some(snos_batch.clone());
                self.check_and_close_batches(&config, &batch, &snos_batch).await?;
            }
        }
        
        let latest_snos_block_in_db = latest_snos_batch.clone().map_or(-1, |batch| batch.end_block as i64);

        // Ensure aggregator and SNOS batches are in sync
        if latest_aggregator_block_in_db != latest_snos_block_in_db {
            return Err(eyre!(
                "Aggregator and SNOS batches are out of sync: aggregator_block={}, snos_block={}",
                latest_aggregator_block_in_db,
                latest_snos_block_in_db
            ));
        }

        // Getting the latest block number from Starknet
        let provider = config.madara_client();
        let block_number_provider = provider.block_number().await?;

        // Calculating the last block number that needs to be assigned to a batch
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(block_number_provider, |max_block| min(max_block, block_number_provider));

        tracing::debug!(latest_block_number = %last_block_to_assign_batch, "Calculated latest block number to batch.");

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block_to_assign_batch =
            max(config.service_config().min_block_to_process, (latest_aggregator_block_in_db + 1) as u64);

        if first_block_to_assign_batch <= last_block_to_assign_batch {
            let (start_block, end_block) =
                self.get_blocks_range_to_process(first_block_to_assign_batch, last_block_to_assign_batch);
            self.assign_batch_to_blocks(start_block, end_block, config.clone()).await?;
        }

        // Releasing the lock
        config.lock().release_lock("BatchingWorker", None).await?;

        tracing::trace!(log_type = "completed", category = "BatchingWorker", "BatchingWorker completed.");
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
                "end_block_number {} is smaller than start_block_number {}",
                end_block_number,
                start_block_number
            ))));
        }
        // Get the database
        let database = config.database();

        // Get the storage client
        let storage = config.storage();

        // Get the latest batch from the database
        let (mut batch, mut state_update) = match database.get_latest_aggregator_batch().await? {
            Some(batch) => {
                // The latest batch is full. Start a new batch
                if batch.is_batch_ready {
                    (self.start_aggregator_batch(&config, batch.index + 1, batch.end_block + 1).await?, None)
                } else {
                    // Previous batch is not full, continue with the previous batch
                    let state_update_bytes = storage.get_data(&batch.squashed_state_updates_path).await?;
                    let state_update: StateUpdate = serde_json::from_slice(&state_update_bytes)?;
                    (batch, Some(state_update))
                }
            }
            None => (
                // No batch in DB. Start a new batch
                self.start_aggregator_batch(&config, 1, start_block_number).await?,
                None,
            ),
        };

        let mut latest_snos_batch = match config.database().get_latest_snos_batch().await? {
            Some(batch) => batch,
            None => SnosBatch::new(1, batch.start_block, batch.end_block),
        };

        // Assign batches to all the blocks
        for block_number in start_block_number..end_block_number + 1 {
            (state_update, batch, latest_snos_batch) = self.assign_batch(block_number, state_update, batch, latest_snos_batch, &config).await?;
        }


        // This just updates the aggregator batch in the DB 
        // and does not actually close the batch
        if let Some(state_update) = state_update {
            self.close_aggregator_batch(
                &batch,
                &state_update,
                false,
                &config,
                end_block_number,
                config.madara_client(),
            )
            .await?;
        }

        // This just updates the SNOS batch in the DB 
        // and does not actually close the batch
        self.close_snos_batch(&latest_snos_batch, &config, Some(SnosBatchStatus::Open)).await?;

        

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
                            if current_aggregator_batch.start_block == 0 { None } else { Some(current_aggregator_batch.start_block - 1) },
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

                            // Close the current batch - store the state update, blob info in storage and update DB
                            self.close_aggregator_batch(
                                &current_aggregator_batch,
                                &prev_state_update,
                                true,
                                config,
                                block_number.saturating_sub(1),
                                provider,
                            )
                            .await?;

                            // If the Aggregator batch is closed, then the SNOS batch should also be closed
                            // Close the SNOS batch
                            self.close_snos_batch(
                                &current_snos_batch,
                                config,
                                Some(SnosBatchStatus::Closed),
                            )
                            .await?;

                            // Start a new batch
                            let new_aggregator_batch =
                                self.start_aggregator_batch(config, current_aggregator_batch.index + 1, block_number).await?;
                            // Starting a new SNOS batch
                            let new_snos_batch = SnosBatch::new(current_snos_batch.index + 1, block_number, block_number);

                            Ok((Some(state_update), new_aggregator_batch, new_snos_batch))
                        } else if self.should_close_snos_batch(config, &current_snos_batch) {
                            // Close the current SNOS batch and start a new one
                            self.close_snos_batch(
                                &current_snos_batch,
                                config,
                                Some(SnosBatchStatus::Closed),
                            )
                            .await?;

                            // Starting a new SNOS batch
                            let new_snos_batch = SnosBatch::new(current_snos_batch.index + 1, block_number, block_number);

                            Ok((Some(state_update), self.update_aggregator_batch_info(current_aggregator_batch, block_number, false).await?, new_snos_batch))
                        } else {
                            // We can add the current block in this batch
                            // Update batch info and return
                            Ok((
                                Some(squashed_state_update),
                                self.update_aggregator_batch_info(current_aggregator_batch, block_number, false).await?,
                                self.update_snos_batch_info(current_snos_batch, block_number).await?,
                            ))
                        }
                    }
                    None => Ok((
                        Some(state_update),
                        self.update_aggregator_batch_info(current_aggregator_batch, block_number, false).await?,
                        self.update_snos_batch_info(current_snos_batch, block_number).await?,
                    )),
                }
            }
            PendingUpdate(_) => {
                tracing::info!("Skipping batching for block {} as it is still pending", block_number);
                Ok((prev_state_update, current_aggregator_batch, current_snos_batch))
            }
        }
    }

    async fn start_aggregator_batch(
        &self,
        config: &Arc<Config>,
        index: u64,
        start_block: u64,
    ) -> Result<AggregatorBatch, JobError> {
        // Start a new bucket
        // let bucket_id = config
        //     .prover_client()
        //     .submit_task(Task::CreateBucket)
        //     .await
        //     .map_err(|e| {
        //         tracing::error!(bucket_index = %index, error = %e, "Failed to submit create bucket task to prover client, {}", e);
        //         JobError::Other(OtherError(eyre!("Prover Client Error: Failed to submit create bucket task to prover client, {}", e))) // TODO: Add a new error type to be used for prover client error
        //     })?;
        let bucket_id = 123;
        tracing::info!(index = %index, bucket_id = %bucket_id, "Created new bucket successfully");
        Ok(AggregatorBatch::new(
            index,
            start_block,
            self.get_state_update_file_path(index),
            self.get_blob_dir_path(index),
            bucket_id.to_string(),
        ))
    }

    /// close_batch stores the state update, blob information in storage, and update DB
    async fn close_aggregator_batch(
        &self,
        batch: &AggregatorBatch,
        state_update: &StateUpdate,
        is_batch_ready: bool,
        config: &Arc<Config>,
        pre_range_block: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<(), JobError> {
        // Get the database
        let database = config.database();

        // Get the storage client
        let storage = config.storage();

        let compressed_state_update =
            self.compress_state_update(state_update, config.params.madara_version, pre_range_block, provider).await?;
        try_join!(
            // Update state update and blob for the batch in storage
            self.store_state_update(storage, state_update, batch),
            self.store_blob(storage, &compressed_state_update, batch),
        )?;

        // Update batch status in the database
        database
            .update_or_create_aggregator_batch(
                batch,
                &AggregatorBatchUpdates {
                    end_block: Some(batch.end_block),
                    is_batch_ready: Some(is_batch_ready),
                    status: if is_batch_ready { Some(AggregatorBatchStatus::Closed) } else { None },
                },
            )
            .await?;

        Ok(())
    }

    async fn close_snos_batch(&self, batch: &SnosBatch, config: &Arc<Config>, status: Option<SnosBatchStatus>) -> Result<(), JobError> {
        let database = config.database();

        database
            .update_or_create_snos_batch(
                batch,
                &SnosBatchUpdates { end_block: Some(batch.end_block), status },
            )
            .await?;

        Ok(())
    }



    /// update_batch_info updates the batch information in the mut Batch argument
    async fn update_aggregator_batch_info(
        &self,
        mut batch: AggregatorBatch,
        end_block: u64,
        is_batch_ready: bool,
    ) -> Result<AggregatorBatch, JobError> {
        batch.end_block = end_block;
        batch.is_batch_ready = is_batch_ready;
        batch.num_blocks = end_block - batch.start_block + 1;
        Ok(batch)
    }

    async fn update_snos_batch_info(
        &self,
        mut batch: SnosBatch,
        end_block: u64,
    ) -> Result<SnosBatch, JobError> {
        batch.end_block = end_block;
        batch.num_blocks = end_block - batch.start_block + 1;
        Ok(batch)
    }

    async fn compress_state_update(
        &self,
        state_update: &StateUpdate,
        madara_version: StarknetVersion,
        pre_range_block: u64,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<Vec<Felt>, JobError> {
        // Perform stateful compression
        let state_update = if madara_version >= StarknetVersion::V0_13_4 {
            stateful_compress(state_update, pre_range_block, provider)
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
        100
    }

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
        (!batch.is_batch_ready)
            && ((state_update_len.is_some() && state_update_len.unwrap() > MAX_BLOB_SIZE)
                || (batch.num_blocks >= config.params.batching_config.max_batch_size)
                || ((Utc::now().round_subsecs(0) - batch.created_at).abs().num_seconds() as u64
                    >= config.params.batching_config.max_batch_time_seconds))
    }

    fn should_close_snos_batch(
        &self,
        #[allow(unused_variables)]
        config: &Arc<Config>,
        batch: &SnosBatch,
    ) -> bool {
        // TODO: Implement this
        batch.end_block % 2 == 0
    }
}
