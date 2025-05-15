use crate::core::config::Config;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{Batch, BatchUpdates};
use crate::types::constant::{MAX_BATCH_SIZE, STORAGE_STATE_UPDATE_DIR};
use crate::worker::event_handler::triggers::JobTrigger;
use bytes::Bytes;
use color_eyre::eyre::eyre;
use starknet::core::types::{
    BlockId, ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, Felt, NonceUpdate, ReplacedClassItem,
    StateDiff, StateUpdate, StorageEntry,
};
use starknet::providers::Provider;
use starknet_core::types::MaybePendingStateUpdate::{PendingUpdate, Update};
use std::cmp::{max, min};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

        // Calculating the latest block number that needs to be assigned to a batch
        let last_block_to_assign_batch = config
            .service_config()
            .max_block_to_process
            .map_or(block_number_provider, |max_block| min(max_block, block_number_provider));

        tracing::debug!(latest_block_number = %last_block_to_assign_batch, "Calculated latest block number to batch.");

        // Getting the latest batch in DB
        let latest_batch = config.database().get_latest_batch().await?;
        let latest_block_in_db = latest_batch.map(|batch| batch.end_block).unwrap_or(0);

        // Calculating the first block number to for which a batch needs to be assigned
        let first_block_to_assign_batch = config
            .service_config()
            .min_block_to_process
            .map_or(latest_block_in_db, |min_block| max(min_block, latest_block_in_db));

        for block_num in first_block_to_assign_batch..last_block_to_assign_batch + 1 {
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
                // FIXME: Update this flow
                // For now, adding a hardcoded limit on the number of blocks each batch can have
                // The Correct implementation:
                // 1. Squash all state updates to get a single state update
                // 2. Perform stateful compression
                // 3. Perform stateless compression
                // 4. Create an array of felts
                // 5. Check if its length is less than 6 * 4096
                //    If yes, it can be added in the same batch
                //    Else, start a new batch
                tracing::info!("Starting batching for block {}", block_number);
                let latest_batch = database.get_latest_batch().await?;
                let mut batch_index = 1;
                if let Some(batch) = latest_batch {
                    // A batch exists
                    // Check if we can add a new block in the same batch
                    if batch.size < MAX_BATCH_SIZE {
                        // Can add in the same batch

                        // Fetch existing state update
                        let current_state_update_bytes = storage.get_data(&batch.squashed_state_updates_path).await?;
                        let current_state_update: StateUpdate = serde_json::from_slice(&current_state_update_bytes)?;
                        // Merge the current block's state update with the batch's state update
                        let new_state_update = self.squash_state_updates(vec![current_state_update, state_update])?;
                        // Update state update for the batch in storage
                        storage
                            .put_data(
                                Bytes::from(serde_json::to_string(&new_state_update)?),
                                &self.get_state_update_file_name(batch.index),
                            )
                            .await?;
                        // Update batch status in the database
                        database
                            .update_batch(&batch, &BatchUpdates { end_block: block_number, is_batch_ready: false })
                            .await?;
                        batch_index = batch.index;
                    } else {
                        // Start a new batch

                        // Update the status of the previous batch
                        database
                            .update_batch(&batch, &BatchUpdates { end_block: batch.end_block, is_batch_ready: true })
                            .await?;
                        let squashed_state_updates_path = self.get_state_update_file_name(batch_index + 1);
                        // Put the state update in storage
                        storage
                            .put_data(Bytes::from(serde_json::to_string(&state_update)?), &squashed_state_updates_path)
                            .await?;
                        // Add the new batch info in the database
                        database
                            .create_batch(Batch::create(batch.index + 1, block_number, squashed_state_updates_path))
                            .await?;
                        batch_index = batch.index + 1;
                    }
                } else {
                    // No batch exists in the DB yet
                    // Create a fresh batch

                    let squashed_state_updates_path = self.get_state_update_file_name(batch_index + 1);
                    // Put the state update in storage
                    storage
                        .put_data(Bytes::from(serde_json::to_string(&state_update)?), &squashed_state_updates_path)
                        .await?;
                    // Add the new batch info in the database
                    database.create_batch(Batch::create(1, block_number, squashed_state_updates_path)).await?;
                    batch_index = 1;
                }
                tracing::info!("Completed batching for block {}. Assigned batch {}", block_number, batch_index);
            }
            PendingUpdate(_) => {
                tracing::info!("Skipping batching for block {} as it is still pending", block_number);
            }
        }

        Ok(())
    }

    fn get_state_update_file_name(&self, batch_index: u64) -> String {
        format!("{}/batch/{}.json", STORAGE_STATE_UPDATE_DIR, batch_index)
    }

    /// squash_state_updates merge all the StateUpdate into a single StateUpdate
    fn squash_state_updates(&self, state_updates: Vec<StateUpdate>) -> Result<StateUpdate, JobError> {
        if state_updates.is_empty() {
            return Err(JobError::Other(OtherError(eyre!("Cannot merge empty state updates"))));
        }

        // Take the last block hash and number from the last update as our "latest"
        let last_update = state_updates.last().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?;
        let block_hash = last_update.block_hash;
        let new_root = last_update.new_root;
        let old_root =
            state_updates.first().ok_or(JobError::Other(OtherError(eyre!("Invalid state updates"))))?.old_root;

        // Create a new StateDiff to hold the merged state
        let mut state_diff = StateDiff {
            storage_diffs: Vec::new(),
            deployed_contracts: Vec::new(),
            declared_classes: Vec::new(),
            deprecated_declared_classes: Vec::new(),
            nonces: Vec::new(),
            replaced_classes: Vec::new(),
        };

        // Maps to efficiently track the latest state
        let mut storage_diffs_map: HashMap<Felt, HashMap<Felt, Felt>> = HashMap::new();
        let mut deployed_contracts_map: HashMap<Felt, Felt> = HashMap::new();
        let mut declared_classes_map: HashMap<Felt, Felt> = HashMap::new();
        let mut nonces_map: HashMap<Felt, Felt> = HashMap::new();
        let mut replaced_classes_map: HashMap<Felt, Felt> = HashMap::new();
        let mut deprecated_classes_set: HashSet<Felt> = HashSet::new();

        // Process each update in order
        for update in state_updates {
            // Process storage diffs
            for contract_diff in update.state_diff.storage_diffs {
                let contract_addr = contract_diff.address;
                let contract_storage_map = storage_diffs_map.entry(contract_addr).or_default();

                for entry in contract_diff.storage_entries {
                    contract_storage_map.insert(entry.key, entry.value);
                }
            }

            // Process deployed contracts
            for item in update.state_diff.deployed_contracts {
                deployed_contracts_map.insert(item.address, item.class_hash);
            }

            // Process declared classes
            for item in update.state_diff.declared_classes {
                declared_classes_map.insert(item.class_hash, item.compiled_class_hash);
            }

            // Process nonces
            for item in update.state_diff.nonces {
                nonces_map.insert(item.contract_address, item.nonce);
            }

            // Process replaced classes
            for item in update.state_diff.replaced_classes {
                replaced_classes_map.insert(item.contract_address, item.class_hash);
            }

            // Process deprecated classes
            for class_hash in update.state_diff.deprecated_declared_classes {
                deprecated_classes_set.insert(class_hash);
            }
        }

        // Convert maps back to the required StateDiff format

        // Storage diffs
        for (contract_addr, storage_map) in storage_diffs_map {
            let storage_entries = storage_map
                .into_iter()
                .filter(|(_, value)| *value != Felt::ZERO) // Filter out entries with value 0x0
                .map(|(key, value)| StorageEntry { key, value })
                .collect::<Vec<_>>();

            // Only include contracts that have non-zero storage entries
            if !storage_entries.is_empty() {
                let contract_storage_diff = ContractStorageDiffItem { address: contract_addr, storage_entries };

                state_diff.storage_diffs.push(contract_storage_diff);
            }
        }

        // Deployed contracts
        state_diff.deployed_contracts = deployed_contracts_map
            .into_iter()
            .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
            .collect();

        // Declared classes
        state_diff.declared_classes = declared_classes_map
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();

        // Nonces
        state_diff.nonces =
            nonces_map.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect();

        // Replaced classes
        state_diff.replaced_classes = replaced_classes_map
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect();

        // Deprecated classes
        state_diff.deprecated_declared_classes = deprecated_classes_set.into_iter().collect();

        // Create the merged StateUpdate
        let merged_update = StateUpdate { block_hash, new_root, old_root, state_diff };

        Ok(merged_update)
    }
}
