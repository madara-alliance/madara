use crate::compression::blob::{convert_felt_vec_to_blob_data, state_update_to_blob_data};
use crate::compression::squash::squash_state_updates;
use crate::compression::stateful::compress as stateful_compress;
use crate::compression::stateless::compress as stateless_compress;
use crate::core::config::{Config, StarknetVersion};
use crate::core::{DatabaseClient, StorageClient};
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::worker::event_handler::jobs::models::{Batch, BatchUpdates};
use crate::worker::event_handler::triggers::JobTrigger;
use bytes::Bytes;
use starknet::core::types::{BlockId, StateUpdate};
use starknet::providers::Provider;
use starknet_core::types::Felt;
use starknet_core::types::MaybePendingStateUpdate::{PendingUpdate, Update};
use std::cmp::{max, min};
use std::sync::Arc;

const MAX_BLOB_SIZE: usize = 4096 * 6;

const STATE_UPDATE_DIR: &str = "state_update";
const BLOB_DIR: &str = "blob";

pub struct BatchingTrigger;

#[async_trait::async_trait]
impl JobTrigger for BatchingTrigger {
    /// 1. Fetch the latest completed block from Starknet chain
    /// 2. Fetch the last batch and check its `end_block`
    /// 3. Assign batches to all the remaining blocks and store the squashed state update in storage
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "BatchingWorker", "BatchingWorker started");

        // Getting the latest block number from Starknet
        let provider = config.madara_client();
        let block_number_provider = provider.block_number().await?;

        // Calculating the last block number to for which a batch needs to be assigned
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(block_number_provider, |max_block| min(max_block, block_number_provider));

        tracing::debug!(latest_block_number = %last_block_to_assign_batch, "Fetched latest block number from starknet");

        // Getting the latest batch in DB
        let latest_batch = config.database().get_latest_batch().await?;
        let latest_block_in_db = latest_batch.map(|batch| batch.start_block).unwrap_or(0);

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block_to_assign_batch = config
            .service_config()
            .min_block_to_process
            .map_or(latest_block_in_db, |min_block| max(min_block, latest_block_in_db));

        for block_num in first_block_to_assign_batch..last_block_to_assign_batch + 1 {
            println!("Assigning batch to block {}", block_num);
            self.assign_batch_to_block(block_num, config.clone()).await?;
        }
        tracing::trace!(log_type = "completed", category = "BatchingWorker", "BatchingWorker completed.");
        Ok(())
    }
}

impl BatchingTrigger {
    /// assign_batch_to_block assigns a batch to the block
    async fn assign_batch_to_block(&self, block_number: u64, config: Arc<Config>) -> Result<(), JobError> {
        // Get the provider
        let provider = config.madara_client();

        // Get the database
        let database = config.database();

        // Get the storage client
        let storage = config.storage();

        // Get the state update for the block
        let state_update = provider
            .get_state_update(BlockId::Number(block_number))
            .await
            .map_err(|e| JobError::ProviderError(e.to_string()))?;

        match state_update {
            Update(state_update) => {
                tracing::info!("Starting batching for block {}", block_number);
                let latest_batch = database.get_latest_batch().await?;
                let assigned_batch_index;
                if let Some(batch) = latest_batch {
                    // A batch exists
                    // Check if we can add a new block in the same batch
                    if !batch.is_batch_ready {
                        // Check if we can add in the same batch

                        // Fetch existing state update
                        let current_state_update_bytes = storage.get_data(&batch.squashed_state_updates_path).await?;
                        let current_state_update: StateUpdate = serde_json::from_slice(&current_state_update_bytes)?;
                        // Merge the current block's state update with the batch's state update
                        let new_state_update = squash_state_updates(
                            vec![current_state_update, state_update.clone()],
                            batch.start_block.saturating_sub(1),
                            provider,
                        )
                        .await?;

                        let compressed_state_update =
                            self.compress_state_update(&new_state_update, config.params.madara_version).await?;

                        eprintln!("Compressed state update size: {}", compressed_state_update.len());

                        if compressed_state_update.len() > MAX_BLOB_SIZE {
                            // We cannot add the current block in this batch

                            // Update the status of the previous batch
                            database
                                .update_batch(
                                    &batch,
                                    &BatchUpdates { end_block: batch.end_block, is_batch_ready: true },
                                )
                                .await?;

                            // Start a new batch with the index `batch_index + 1`
                            assigned_batch_index = batch.index + 1;
                            self.start_new_batch(
                                storage,
                                database,
                                assigned_batch_index,
                                block_number,
                                &state_update,
                                &compressed_state_update,
                            )
                            .await?
                        } else {
                            // We can add the current block in this batch

                            assigned_batch_index = batch.index;
                            self.update_batch(
                                storage,
                                database,
                                &new_state_update,
                                &compressed_state_update,
                                &batch,
                                block_number,
                                false,
                            )
                            .await?
                        }
                    } else {
                        // The previous block is full
                        // Start a new batch

                        assigned_batch_index = batch.index + 1;
                        self.start_new_batch(
                            storage,
                            database,
                            assigned_batch_index,
                            block_number,
                            &state_update,
                            &self.compress_state_update(&state_update, config.params.madara_version).await?,
                        )
                        .await?
                    }
                } else {
                    // No batch exists in the DB yet
                    // Create the first batch

                    assigned_batch_index = 1;
                    self.start_new_batch(
                        storage,
                        database,
                        assigned_batch_index,
                        block_number,
                        &state_update,
                        &self.compress_state_update(&state_update, config.params.madara_version).await?,
                    )
                    .await?
                }
                tracing::info!(
                    "Completed batching for block {}. Assigned batch {}",
                    block_number,
                    assigned_batch_index
                );
            }
            PendingUpdate(_) => {
                tracing::info!("Skipping batching for block {} as it is still pending", block_number);
            }
        }

        Ok(())
    }

    async fn compress_state_update(
        &self,
        state_update: &StateUpdate,
        madara_version: StarknetVersion,
    ) -> Result<Vec<Felt>, JobError> {
        // Perform stateful compression
        let stateful_compressed = stateful_compress(state_update).map_err(|err| JobError::Other(OtherError(err)))?;
        // Get a vector of felts from the compressed state update
        let vec_felts = state_update_to_blob_data(stateful_compressed, madara_version).await?;
        // Perform stateless compression
        Ok(stateless_compress(&vec_felts))
    }

    /// get_state_update_file_name returns the file path for storing the state update in storage
    fn get_state_update_file_name(&self, batch_index: u64) -> String {
        format!("{}/batch/{}.json", STATE_UPDATE_DIR, batch_index)
    }

    fn get_blob_file_name(&self, batch_index: u64) -> String {
        format!("{}/batch/{}.txt", BLOB_DIR, batch_index)
    }

    /// start_new_batch starts a new batch
    async fn start_new_batch(
        &self,
        storage: &dyn StorageClient,
        database: &dyn DatabaseClient,
        batch_index: u64,
        start_block: u64,
        state_update: &StateUpdate,
        compressed_state_update: &Vec<Felt>,
    ) -> Result<(), JobError> {
        // Create a new batch
        let batch = Batch::create(batch_index, start_block, self.get_state_update_file_name(batch_index), self.get_blob_file_name(batch_index));
        // Put the state update and blob in storage
        self.store_state_update(storage, state_update, &batch).await?;
        self.store_blob(storage, compressed_state_update, &batch).await?;
        // Add the new batch info in the database
        database.create_batch(batch).await?;
        Ok(())
    }

    async fn update_batch(
        &self,
        storage: &dyn StorageClient,
        database: &dyn DatabaseClient,
        state_update: &StateUpdate,
        compressed_state_update: &Vec<Felt>,
        batch: &Batch,
        end_block: u64,
        is_batch_ready: bool,
    ) -> Result<(), JobError> {
        // Update state update and blob for the batch in storage
        self.store_state_update(storage, &state_update, batch).await?;
        self.store_blob(storage, compressed_state_update, batch).await?;
        // Update batch status in the database
        database.update_batch(batch, &BatchUpdates { end_block, is_batch_ready }).await?;
        Ok(())
    }

    async fn store_state_update(
        &self,
        storage: &dyn StorageClient,
        state_update: &StateUpdate,
        batch: &Batch,
    ) -> Result<(), JobError> {
        storage
            .put_data(Bytes::from(serde_json::to_string(&state_update)?), &self.get_state_update_file_name(batch.index))
            .await?;
        Ok(())
    }

    async fn store_blob(
        &self,
        storage: &dyn StorageClient,
        compressed_state_update: &Vec<Felt>,
        batch: &Batch,
    ) -> Result<(), JobError> {
        storage
            .put_data(
                Bytes::from(convert_felt_vec_to_blob_data(compressed_state_update)),
                &self.get_blob_file_name(batch.index),
            )
            .await?;
        Ok(())
    }
}
