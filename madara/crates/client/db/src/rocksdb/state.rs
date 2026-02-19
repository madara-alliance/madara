use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE},
};
use mp_convert::Felt;
use mp_state_update::{
    ContractStorageDiffItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff, StorageEntry,
};
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
use rocksdb::{IteratorMode, ReadOptions};
use std::collections::HashMap;

// TODO: Remove the need for this struct.
#[derive(Debug)]
struct ContractDbBlockUpdate {
    contract_class_updates: Vec<(Felt, Felt)>,
    contract_nonces_updates: Vec<(Felt, Felt)>,
    contract_kv_updates: Vec<((Felt, Felt), Felt)>,
}

impl ContractDbBlockUpdate {
    fn from_state_diff(state_diff: StateDiff) -> Self {
        let nonces_from_updates =
            state_diff.nonces.into_iter().map(|NonceUpdate { contract_address, nonce }| (contract_address, nonce));

        let nonce_map: HashMap<Felt, Felt> = nonces_from_updates.collect();

        let contract_class_updates_replaced = state_diff
            .replaced_classes
            .into_iter()
            .map(|ReplacedClassItem { contract_address, class_hash }| (contract_address, class_hash));

        let contract_class_updates_deployed = state_diff
            .deployed_contracts
            .into_iter()
            .map(|DeployedContractItem { address, class_hash }| (address, class_hash));

        let contract_class_updates =
            contract_class_updates_replaced.chain(contract_class_updates_deployed).collect::<Vec<_>>();
        let contract_nonces_updates = nonce_map.into_iter().collect::<Vec<_>>();

        let contract_kv_updates = state_diff
            .storage_diffs
            .into_iter()
            .flat_map(|ContractStorageDiffItem { address, storage_entries }| {
                storage_entries.into_iter().map(move |StorageEntry { key, value }| ((address, key), value))
            })
            .collect::<Vec<_>>();

        Self { contract_class_updates, contract_nonces_updates, contract_kv_updates }
    }
}

// NB: Columns cf needs prefix extractor of these length during creation
const CONTRACT_STORAGE_PREFIX_LEN: usize = 64;
const CONTRACT_NONCES_PREFIX_LEN: usize = 32;
const CONTRACT_CLASS_HASH_PREFIX_LEN: usize = 32;

pub(crate) const CONTRACT_STORAGE_COLUMN: Column =
    Column::new("contract_storage").with_prefix_extractor_len(CONTRACT_STORAGE_PREFIX_LEN).use_contracts_mem_budget();
pub(crate) const CONTRACT_NONCE_COLUMN: Column =
    Column::new("contract_nonces").with_prefix_extractor_len(CONTRACT_NONCES_PREFIX_LEN).use_contracts_mem_budget();
pub(crate) const CONTRACT_CLASS_HASH_COLUMN: Column = Column::new("contract_class_hashes")
    .with_prefix_extractor_len(CONTRACT_CLASS_HASH_PREFIX_LEN)
    .use_contracts_mem_budget();

// prefix [<contract_address 32 bytes> | <storage_key 32 bytes>] | <block_n 4 bytes>
const STORAGE_KEY_LEN: usize = 32 * 2 + size_of::<u32>();
fn make_storage_column_key(contract_address: &Felt, storage_key: &Felt, block_n: u32) -> [u8; STORAGE_KEY_LEN] {
    let mut key = [0u8; STORAGE_KEY_LEN];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..2 * 32].copy_from_slice(&storage_key.to_bytes_be());
    // Reverse block_n order to iterate forwards.
    key[2 * 32..].copy_from_slice(&(u32::MAX - block_n).to_be_bytes());
    key
}

// prefix [<contract_address 32 bytes>] | <block_n 4 bytes>
const CONTRACT_KEY_LEN: usize = 32 + size_of::<u32>();
fn make_contract_column_key(contract_address: &Felt, block_n: u32) -> [u8; CONTRACT_KEY_LEN] {
    let mut key = [0u8; CONTRACT_KEY_LEN];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    // Reverse block_n order to iterate forwards.
    key[32..].copy_from_slice(&(u32::MAX - block_n).to_be_bytes());
    key
}

impl RocksDBStorageInner {
    fn db_history_kv_resolve<V: serde::de::DeserializeOwned + 'static>(
        &self,
        bin_prefix: &[u8],
        col: Column,
    ) -> Result<Option<V>> {
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let mode = IteratorMode::From(bin_prefix, rocksdb::Direction::Forward); // Iterate forward since we reversed block_n order (this is faster)
        let mut iter = DBIterator::new_cf(&self.db, &self.get_column(col), options, mode)
            .into_iter_values(|bytes| super::deserialize(bytes));
        let n = iter.next();

        Ok(n.transpose()?.transpose()?)
    }

    fn db_history_kv_contains(&self, bin_prefix: &[u8], col: Column) -> Result<bool> {
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let mode = IteratorMode::From(bin_prefix, rocksdb::Direction::Forward); // Iterate forward since we reversed block_n order (this is faster)
        let mut iter = DBIterator::new_cf(&self.db, &self.get_column(col), options, mode);
        Ok(iter.next()?)
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        let block_n = u32::try_from(block_n).unwrap_or(u32::MAX); // We can't store blocks past u32::MAX.
        let prefix = make_storage_column_key(contract_address, key, block_n);
        self.db_history_kv_resolve(&prefix, CONTRACT_STORAGE_COLUMN)
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        let block_n = u32::try_from(block_n).unwrap_or(u32::MAX); // We can't store blocks past u32::MAX.
        let prefix = make_contract_column_key(contract_address, block_n);
        self.db_history_kv_resolve(&prefix, CONTRACT_NONCE_COLUMN)
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        let block_n = u32::try_from(block_n).unwrap_or(u32::MAX); // We can't store blocks past u32::MAX.
        let prefix = make_contract_column_key(contract_address, block_n);
        self.db_history_kv_resolve(&prefix, CONTRACT_CLASS_HASH_COLUMN)
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool> {
        let block_n = u32::try_from(block_n).unwrap_or(u32::MAX); // We can't store blocks past u32::MAX.
        let prefix = make_contract_column_key(contract_address, block_n);
        self.db_history_kv_contains(&prefix, CONTRACT_CLASS_HASH_COLUMN)
    }

    #[tracing::instrument(skip(self, value))]
    pub(crate) fn state_apply_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        let value = ContractDbBlockUpdate::from_state_diff(value.clone());
        let block_n = u32::try_from(block_n).context("Converting block_n to u32")?;

        value.contract_class_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.get_column(CONTRACT_CLASS_HASH_COLUMN),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (contract_address, value) in chunk {
                    batch.put_cf(
                        col,
                        make_contract_column_key(contract_address, block_n),
                        super::serialize_to_smallvec::<[u8; 64]>(&value)?,
                    );
                }
                self.db.write_opt(batch, &self.writeopts)?;
                anyhow::Ok(())
            },
        )?;
        value.contract_nonces_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.get_column(CONTRACT_NONCE_COLUMN),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (contract_address, value) in chunk {
                    batch.put_cf(
                        col,
                        make_contract_column_key(contract_address, block_n),
                        super::serialize_to_smallvec::<[u8; 64]>(&value)?,
                    );
                }
                self.db.write_opt(batch, &self.writeopts)?;
                anyhow::Ok(())
            },
        )?;
        value.contract_kv_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.get_column(CONTRACT_STORAGE_COLUMN),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for ((contract_address, key), value) in chunk {
                    batch.put_cf(
                        col,
                        make_storage_column_key(contract_address, key, block_n),
                        super::serialize_to_smallvec::<[u8; 64]>(&value)?,
                    );
                }
                self.db.write_opt(batch, &self.writeopts)?;
                anyhow::Ok(())
            },
        )?;

        Ok(())
    }

    pub(crate) fn state_remove(
        &self,
        block_n: u64,
        value: &StateDiff,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let value = ContractDbBlockUpdate::from_state_diff(value.clone());

        let contract_class_hash_col = self.get_column(CONTRACT_CLASS_HASH_COLUMN);
        let contract_nonce_col = self.get_column(CONTRACT_NONCE_COLUMN);
        let contract_storage_col = self.get_column(CONTRACT_STORAGE_COLUMN);

        let block_n_u32 = u32::try_from(block_n).context("Converting block_n to u32")?;

        // Class hashes
        for (contract_address, _class_hash) in &value.contract_class_updates {
            batch.delete_cf(&contract_class_hash_col, make_contract_column_key(contract_address, block_n_u32));
        }

        // Nonces
        for (contract_address, _nonce) in &value.contract_nonces_updates {
            batch.delete_cf(&contract_nonce_col, make_contract_column_key(contract_address, block_n_u32));
        }

        // Contract storage (kv)
        for ((contract_address, key), _value) in &value.contract_kv_updates {
            batch.delete_cf(&contract_storage_col, make_storage_column_key(contract_address, key, block_n_u32));
        }

        Ok(())
    }

    /// Revert items in the contract db.
    ///
    /// `state_diffs` should be a Vec of tuples containing the block number and the entire StateDiff
    /// to be reverted in that block.
    ///
    /// **Warning:** While not enforced, the following should be true:
    ///  * Each `StateDiff` should include the entire state for its block
    ///  * `state_diffs` should form a contiguous range of blocks
    ///  * that range should end with the current blockchain tip
    ///
    /// If this isn't the case, the db could end up storing inconsistent state for some blocks.
    #[tracing::instrument(skip(self, state_diffs))]
    pub(super) fn contract_db_revert(&self, state_diffs: &[(u64, StateDiff)]) -> Result<()> {
        tracing::info!("📝 REORG [contract_db_revert]: Starting with {} state diffs", state_diffs.len());

        if state_diffs.is_empty() {
            tracing::info!("📝 REORG [contract_db_revert]: No state diffs to process, skipping");
            return Ok(());
        }

        let mut batch = WriteBatchWithTransaction::default();
        let mut total_deployed = 0;
        let mut total_replaced = 0;
        let mut total_nonces = 0;
        let mut total_storage_entries = 0;

        for (block_n, diff) in state_diffs {
            tracing::debug!(
                "📝 REORG [contract_db_revert]: Processing block {} with {} deployed, {} replaced, {} nonces, {} storage diffs",
                block_n,
                diff.deployed_contracts.len(),
                diff.replaced_classes.len(),
                diff.nonces.len(),
                diff.storage_diffs.len()
            );

            // Reuse existing state_remove logic
            self.state_remove(*block_n, diff, &mut batch)?;

            // Track totals for logging
            total_deployed += diff.deployed_contracts.len();
            total_replaced += diff.replaced_classes.len();
            total_nonces += diff.nonces.len();
            total_storage_entries += diff.storage_diffs.iter().map(|sd| sd.storage_entries.len()).sum::<usize>();
        }

        tracing::info!(
            "📝 REORG [contract_db_revert]: Removing {} deployed contracts, {} replaced classes, {} nonces, {} storage entries",
            total_deployed,
            total_replaced,
            total_nonces,
            total_storage_entries
        );

        self.db.write_opt(batch, &self.writeopts)?;

        tracing::info!("✅ REORG [contract_db_revert]: Successfully removed all contract state");

        Ok(())
    }
}
