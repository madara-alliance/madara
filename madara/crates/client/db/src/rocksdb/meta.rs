use crate::{
    preconfirmed::PreconfirmedExecutedTransaction,
    prelude::*,
    rocksdb::{
        iter_pinned::DBIterator,
        preconfirmed::{preconfirmed_tx_key, preconfirmed_tx_prefix, PRECONFIRMED_COLUMN},
        Column, RocksDBStorageInner, WriteBatchWithTransaction,
    },
    storage::{DevnetPredeployedKeys, StorageChainTip, StoredChainInfo},
};
use mp_block::header::PreconfirmedHeader;
use mp_chain_config::{ChainConfig, RuntimeExecutionConfig, RuntimeExecutionConfigSerializable};
use rocksdb::{IteratorMode, ReadOptions};

pub const META_COLUMN: Column = Column::new("meta").set_point_lookup();

const META_DEVNET_KEYS_KEY: &[u8] = b"DEVNET_KEYS";
const META_LAST_SYNCED_L1_EVENT_BLOCK_KEY: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
const META_CONFIRMED_ON_L1_TIP_KEY: &[u8] = b"CONFIRMED_ON_L1_TIP";
const META_CHAIN_TIP_KEY: &[u8] = b"CHAIN_TIP";
const META_CHAIN_INFO_KEY: &[u8] = b"CHAIN_INFO";
const META_LATEST_APPLIED_TRIE_UPDATE: &[u8] = b"LATEST_APPLIED_TRIE_UPDATE";
const META_RUNTIME_EXEC_CONFIG_KEY: &[u8] = b"RUNTIME_EXEC_CONFIG";
const META_SNAP_SYNC_LATEST_BLOCK: &[u8] = b"SNAP_SYNC_LATEST_BLOCK";

// Parallel-merkle staged/checkpoint metadata (kept in META_COLUMN).
const META_PARALLEL_MERKLE_STAGED_STATE_PREFIX: &[u8] = b"PARALLEL_MERKLE_STAGED_STATE/";
const META_PARALLEL_MERKLE_CHECKPOINT_PREFIX: &[u8] = b"PARALLEL_MERKLE_CHECKPOINT/";
const META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY: &[u8] = b"PARALLEL_MERKLE_LATEST_CHECKPOINT";

fn meta_key_with_block_n(prefix: &[u8], block_n: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + size_of::<u64>());
    key.extend_from_slice(prefix);
    key.extend_from_slice(&block_n.to_be_bytes());
    key
}

fn parse_meta_block_n_key(prefix: &[u8], key: &[u8]) -> Option<u64> {
    let suffix = key.strip_prefix(prefix)?;
    let bytes: [u8; size_of::<u64>()] = suffix.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

#[derive(serde::Deserialize, serde::Serialize)]
pub enum StoredChainTipWithoutContent {
    Confirmed(u64),
    Preconfirmed(PreconfirmedHeader),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ParallelMerkleStagedState {
    Txns,
    Diff,
    Final,
}

impl RocksDBStorageInner {
    fn get_meta_prefixed<T: serde::de::DeserializeOwned>(&self, prefix: &[u8], block_n: u64) -> Result<Option<T>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), meta_key_with_block_n(prefix, block_n))?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    fn has_meta_prefixed(&self, prefix: &[u8], block_n: u64) -> Result<bool> {
        Ok(self.db.get_pinned_cf(&self.get_column(META_COLUMN), meta_key_with_block_n(prefix, block_n))?.is_some())
    }

    pub(super) fn replace_chain_tip_with_confirmed_in_batch(
        &self,
        block_n: u64,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let meta_col = self.get_column(META_COLUMN);

        if let Some(StoredChainTipWithoutContent::Preconfirmed(prev_header)) = self.get_chain_tip_without_content()? {
            self.preconfirmed_delete_block_txs_in_batch(prev_header.block_number, batch);
        }
        batch.put_cf(
            &meta_col,
            META_CHAIN_TIP_KEY,
            super::serialize_to_smallvec::<[u8; 128]>(&StoredChainTipWithoutContent::Confirmed(block_n))?,
        );

        Ok(())
    }

    pub(super) fn parallel_merkle_set_staged_state(
        &self,
        block_n: u64,
        state: ParallelMerkleStagedState,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let meta_col = self.get_column(META_COLUMN);
        batch.put_cf(
            &meta_col,
            meta_key_with_block_n(META_PARALLEL_MERKLE_STAGED_STATE_PREFIX, block_n),
            super::serialize_to_smallvec::<[u8; 8]>(&state)?,
        );
        Ok(())
    }

    pub(super) fn parallel_merkle_clear_staged_block(&self, block_n: u64, batch: &mut WriteBatchWithTransaction) {
        let meta_col = self.get_column(META_COLUMN);
        batch.delete_cf(&meta_col, meta_key_with_block_n(META_PARALLEL_MERKLE_STAGED_STATE_PREFIX, block_n));
        self.parallel_merkle_clear_staged_header(block_n, batch);
    }

    pub(super) fn parallel_merkle_has_staged_block(&self, block_n: u64) -> Result<bool> {
        Ok(matches!(self.parallel_merkle_get_staged_state(block_n)?, Some(ParallelMerkleStagedState::Final)))
    }

    pub(super) fn parallel_merkle_get_staged_state(&self, block_n: u64) -> Result<Option<ParallelMerkleStagedState>> {
        self.get_meta_prefixed(META_PARALLEL_MERKLE_STAGED_STATE_PREFIX, block_n)
    }

    pub(super) fn parallel_merkle_get_staged_blocks(&self) -> Result<Vec<u64>> {
        let meta_col = self.get_column(META_COLUMN);
        let start = META_PARALLEL_MERKLE_STAGED_STATE_PREFIX;
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(false);
        let mut out = Vec::new();

        for item in
            DBIterator::new_cf(&self.db, &meta_col, options, IteratorMode::From(start, rocksdb::Direction::Forward))
                .into_iter_items(|(key, value)| (key.to_vec(), value.to_vec()))
        {
            let (key, value) = item?;
            if !key.starts_with(start) {
                break;
            }
            if let Some(block_n) = parse_meta_block_n_key(start, key.as_slice()) {
                if matches!(
                    super::deserialize::<ParallelMerkleStagedState>(&value),
                    Ok(ParallelMerkleStagedState::Final)
                ) {
                    out.push(block_n);
                }
            }
        }

        Ok(out)
    }

    pub(super) fn write_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        self.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    pub(super) fn parallel_merkle_mark_checkpoint_in_batch(
        &self,
        block_n: u64,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        if let Some(latest_checkpoint) = self.get_parallel_merkle_latest_checkpoint()? {
            if block_n < latest_checkpoint {
                anyhow::bail!(
                    "parallel merkle checkpoint must be monotonic: latest={latest_checkpoint}, attempted={block_n}"
                );
            }
        }

        let meta_col = self.get_column(META_COLUMN);
        batch.put_cf(&meta_col, meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, block_n), [1u8]);
        batch.put_cf(
            &meta_col,
            META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY,
            super::serialize_to_smallvec::<[u8; 16]>(&block_n)?,
        );
        Ok(())
    }

    pub(super) fn has_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<bool> {
        self.has_meta_prefixed(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, block_n)
    }

    pub(super) fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY)?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    fn latest_meta_block_n_at_or_before(&self, prefix: &[u8], target_block_n: u64) -> Result<Option<u64>> {
        let meta_col = self.get_column(META_COLUMN);
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(false);
        let mut latest = None;

        for item in
            DBIterator::new_cf(&self.db, &meta_col, options, IteratorMode::From(prefix, rocksdb::Direction::Forward))
                .into_iter_items(|(key, _value)| key.to_vec())
        {
            let key = item?;
            if !key.starts_with(prefix) {
                break;
            }
            let Some(block_n) = parse_meta_block_n_key(prefix, key.as_slice()) else {
                continue;
            };
            if block_n > target_block_n {
                break;
            }
            latest = Some(block_n);
        }

        Ok(latest)
    }

    fn delete_meta_entries_above(
        &self,
        prefix: &[u8],
        target_block_n: u64,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let meta_col = self.get_column(META_COLUMN);
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(false);

        for item in
            DBIterator::new_cf(&self.db, &meta_col, options, IteratorMode::From(prefix, rocksdb::Direction::Forward))
                .into_iter_items(|(key, _value)| key.to_vec())
        {
            let key = item?;
            if !key.starts_with(prefix) {
                break;
            }

            if let Some(block_n) = parse_meta_block_n_key(prefix, key.as_slice()) {
                if block_n > target_block_n {
                    batch.delete_cf(&meta_col, key);
                }
            }
        }

        Ok(())
    }

    /// On chain reorg, remove stale staged/checkpoint metadata above target and clamp latest checkpoint.
    pub(super) fn parallel_merkle_reorg_metadata_to(&self, target_block_n: u64) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let meta_col = self.get_column(META_COLUMN);

        self.delete_meta_entries_above(META_PARALLEL_MERKLE_STAGED_STATE_PREFIX, target_block_n, &mut batch)?;
        self.delete_meta_entries_above(META_PARALLEL_MERKLE_STAGED_HEADER_PREFIX, target_block_n, &mut batch)?;
        self.delete_meta_entries_above(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, target_block_n, &mut batch)?;

        if let Some(latest_checkpoint) =
            self.latest_meta_block_n_at_or_before(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, target_block_n)?
        {
            batch.put_cf(
                &meta_col,
                META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY,
                super::serialize_to_smallvec::<[u8; 16]>(&latest_checkpoint)?,
            );
        } else {
            batch.delete_cf(&meta_col, META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY);
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    /// Set the latest l1_block synced for the messaging worker.
    #[tracing::instrument(skip(self))]
    pub(super) fn write_l1_messaging_sync_tip(&self, block_n: Option<u64>) -> Result<()> {
        if let Some(block_n) = block_n {
            self.db.put_cf_opt(
                &self.get_column(META_COLUMN),
                META_LAST_SYNCED_L1_EVENT_BLOCK_KEY,
                block_n.to_be_bytes(),
                &self.writeopts,
            )?;
        } else {
            self.db.delete_cf_opt(
                &self.get_column(META_COLUMN),
                META_LAST_SYNCED_L1_EVENT_BLOCK_KEY,
                &self.writeopts,
            )?;
        }
        Ok(())
    }

    /// Get the latest l1_block synced for the messaging worker.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        let Some(data) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_LAST_SYNCED_L1_EVENT_BLOCK_KEY)?
        else {
            return Ok(None);
        };
        Ok(Some(u64::from_be_bytes(data[..].try_into().context("Malformated block_n in DB")?)))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn write_confirmed_on_l1_tip(&self, block_n: Option<u64>) -> Result<()> {
        if let Some(block_n) = block_n {
            self.db.put_cf_opt(
                &self.get_column(META_COLUMN),
                META_CONFIRMED_ON_L1_TIP_KEY,
                block_n.to_be_bytes(),
                &self.writeopts,
            )?;
        } else {
            self.db.delete_cf_opt(&self.get_column(META_COLUMN), META_CONFIRMED_ON_L1_TIP_KEY, &self.writeopts)?;
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        let Some(data) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_CONFIRMED_ON_L1_TIP_KEY)? else {
            return Ok(None);
        };
        Ok(Some(u64::from_be_bytes(data[..].try_into().context("Malformated block_n in DB")?)))
    }

    /// Get the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_DEVNET_KEYS_KEY)? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    /// Set the devnet predeployed contracts keys.
    #[tracing::instrument(skip(self, devnet_keys))]
    pub(super) fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_DEVNET_KEYS_KEY,
            super::serialize(&devnet_keys)?,
            &self.writeopts,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn replace_chain_tip(&self, chain_tip: &StorageChainTip) -> Result<()> {
        // We need to do all of this in a single write. (atomic)

        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);
        let meta_col = self.get_column(META_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();

        // Delete previous preconfirmed content for the previous preconfirmed tip (if any).
        if let Some(StoredChainTipWithoutContent::Preconfirmed(prev_header)) = self.get_chain_tip_without_content()? {
            self.preconfirmed_delete_block_txs_in_batch(prev_header.block_number, &mut batch);
        }

        // Write new chain tip.
        match chain_tip {
            StorageChainTip::Empty => batch.delete_cf(&meta_col, META_CHAIN_TIP_KEY),
            StorageChainTip::Confirmed(block_n) => {
                batch.put_cf(
                    &meta_col,
                    META_CHAIN_TIP_KEY,
                    super::serialize_to_smallvec::<[u8; 128]>(&StoredChainTipWithoutContent::Confirmed(*block_n))?,
                );
            }
            StorageChainTip::Preconfirmed { header, content } => {
                batch.put_cf(
                    &meta_col,
                    META_CHAIN_TIP_KEY,
                    super::serialize_to_smallvec::<[u8; 128]>(&StoredChainTipWithoutContent::Preconfirmed(
                        header.clone(),
                    ))?,
                );
                // Write new preconfirmed content.
                for (tx_index, val) in content.iter().enumerate() {
                    let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
                    batch.put_cf(
                        &preconfirmed_col,
                        preconfirmed_tx_key(header.block_number, tx_index),
                        super::serialize(&val)?,
                    );
                }
            }
        };

        // Write chain tip atomically
        // Note: Using regular write opts (no fsync) for performance
        // The chain tip will be synced on next flush or graceful shutdown
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    // internal utility method
    #[tracing::instrument(skip(self))]
    pub(super) fn get_chain_tip_without_content(&self) -> Result<Option<StoredChainTipWithoutContent>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_CHAIN_TIP_KEY)? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize::<StoredChainTipWithoutContent>(&res)?))
    }

    #[tracing::instrument(skip(self, txs))]
    pub(super) fn append_preconfirmed_content(
        &self,
        block_n: u64,
        start_tx_index: u64,
        txs: &[PreconfirmedExecutedTransaction],
    ) -> Result<()> {
        let col = self.get_column(PRECONFIRMED_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();
        for (i, value) in txs.iter().enumerate() {
            let tx_index = start_tx_index + i as u64;
            let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(&col, preconfirmed_tx_key(block_n, tx_index), super::serialize(&value)?);
        }
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_chain_tip(&self) -> Result<StorageChainTip> {
        match self.get_chain_tip_without_content()? {
            None => Ok(StorageChainTip::Empty),
            Some(StoredChainTipWithoutContent::Confirmed(block_n)) => Ok(StorageChainTip::Confirmed(block_n)),
            Some(StoredChainTipWithoutContent::Preconfirmed(header)) => {
                let start = preconfirmed_tx_prefix(header.block_number);
                let tx_prefix = preconfirmed_tx_prefix(header.block_number);
                let content: Vec<PreconfirmedExecutedTransaction> = DBIterator::new_cf(
                    &self.db,
                    &self.get_column(PRECONFIRMED_COLUMN),
                    ReadOptions::default(),
                    IteratorMode::From(&start, rocksdb::Direction::Forward),
                )
                .into_iter_items(|(key, value)| (key.to_vec(), value.to_vec()))
                .take_while(|item| item.as_ref().map(|(key, _)| key.starts_with(&tx_prefix)).unwrap_or(true))
                .map(|res| -> Result<PreconfirmedExecutedTransaction> {
                    let (_, bytes) = res?;
                    Ok(super::deserialize::<PreconfirmedExecutedTransaction>(&bytes)?)
                })
                .collect::<Result<_>>()?;

                Ok(StorageChainTip::Preconfirmed { header, content })
            }
        }
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_CHAIN_INFO_KEY)? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn write_chain_info(&self, info: &StoredChainInfo) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_CHAIN_INFO_KEY,
            super::serialize_to_smallvec::<[u8; 128]>(info)?,
            &self.writeopts,
        )?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_LATEST_APPLIED_TRIE_UPDATE)? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        if let Some(block_n) = block_n {
            self.db.put_cf_opt(
                &self.get_column(META_COLUMN),
                META_LATEST_APPLIED_TRIE_UPDATE,
                super::serialize_to_smallvec::<[u8; 128]>(block_n)?,
                &self.writeopts,
            )?;
        } else {
            self.db.delete_cf_opt(&self.get_column(META_COLUMN), META_LATEST_APPLIED_TRIE_UPDATE, &self.writeopts)?;
        }
        Ok(())
    }

    /// Write the runtime execution configuration to the database.
    #[tracing::instrument(skip(self, config))]
    pub(super) fn write_runtime_exec_config(&self, config: &RuntimeExecutionConfig) -> Result<()> {
        let serializable = config.to_serializable()?;
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            META_RUNTIME_EXEC_CONFIG_KEY,
            super::serialize(&serializable)?,
            &self.writeopts,
        )?;
        Ok(())
    }

    /// Get the runtime execution configuration from the database.
    /// Note: This requires a backend_chain_config to reconstruct the full ChainConfig.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_runtime_exec_config(
        &self,
        backend_chain_config: &ChainConfig,
    ) -> Result<Option<RuntimeExecutionConfig>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_RUNTIME_EXEC_CONFIG_KEY)? else {
            return Ok(None);
        };
        let serializable: RuntimeExecutionConfigSerializable = super::deserialize(&res)?;
        Ok(Some(RuntimeExecutionConfig::from_saved_config(serializable, backend_chain_config)?))
    }

    /// Get the latest block number where snap sync computed the trie.
    /// Returns None if snap sync was never used.
    #[tracing::instrument(skip(self))]
    pub(super) fn get_snap_sync_latest_block(&self) -> Result<Option<u64>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_SNAP_SYNC_LATEST_BLOCK)? else {
            tracing::debug!("📖 Reading snap_sync_latest_block: None (snap sync never used)");
            return Ok(None);
        };
        let block_n = super::deserialize(&res)?;
        tracing::debug!("📖 Reading snap_sync_latest_block: Some({})", block_n);
        Ok(Some(block_n))
    }

    /// Set the latest block number where snap sync computed the trie.
    #[tracing::instrument(skip(self))]
    pub(super) fn write_snap_sync_latest_block(&self, block_n: &Option<u64>) -> Result<()> {
        if let Some(block_n) = block_n {
            tracing::debug!("✍️  Setting snap_sync_latest_block to: {}", block_n);
            self.db.put_cf_opt(
                &self.get_column(META_COLUMN),
                META_SNAP_SYNC_LATEST_BLOCK,
                super::serialize_to_smallvec::<[u8; 128]>(block_n)?,
                &self.writeopts,
            )?;
        } else {
            tracing::debug!("🗑️  Clearing snap_sync_latest_block (set to None)");
            self.db.delete_cf_opt(&self.get_column(META_COLUMN), META_SNAP_SYNC_LATEST_BLOCK, &self.writeopts)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::{RocksDBConfig, RocksDBStorage};
    use mp_block::header::PreconfirmedHeader;

    fn create_test_storage() -> (tempfile::TempDir, RocksDBStorage) {
        let temp_dir = tempfile::TempDir::with_prefix("meta-test").unwrap();
        let storage = RocksDBStorage::open(temp_dir.path(), RocksDBConfig::default()).unwrap();
        (temp_dir, storage)
    }

    fn mark_staged_block(storage: &RocksDBStorage, block_n: u64) {
        let mut batch = WriteBatchWithTransaction::default();
        let header = PreconfirmedHeader { block_number: block_n, ..Default::default() };
        storage.inner.parallel_merkle_set_staged_header(block_n, &header, &mut batch).unwrap();
        storage.inner.parallel_merkle_set_staged_state(block_n, ParallelMerkleStagedState::Final, &mut batch).unwrap();
        storage.inner.db.write_opt(batch, &storage.inner.writeopts).unwrap();
    }

    #[test]
    fn parallel_merkle_reorg_metadata_prunes_entries_above_target() {
        let (_tmp, storage) = create_test_storage();

        storage.inner.write_parallel_merkle_checkpoint(5).unwrap();
        storage.inner.write_parallel_merkle_checkpoint(8).unwrap();
        mark_staged_block(&storage, 9);

        storage.inner.parallel_merkle_reorg_metadata_to(6).unwrap();

        assert!(storage.inner.has_parallel_merkle_checkpoint(5).unwrap());
        assert!(!storage.inner.has_parallel_merkle_checkpoint(8).unwrap());
        assert_eq!(storage.inner.get_parallel_merkle_latest_checkpoint().unwrap(), Some(5));
        assert!(!storage.inner.has_parallel_merkle_staged_block(9).unwrap());
    }

    #[test]
    fn parallel_merkle_reorg_metadata_clears_latest_checkpoint_when_no_checkpoint_remains() {
        let (_tmp, storage) = create_test_storage();

        storage.inner.write_parallel_merkle_checkpoint(7).unwrap();
        storage.inner.write_parallel_merkle_checkpoint(9).unwrap();

        storage.inner.parallel_merkle_reorg_metadata_to(3).unwrap();

        assert_eq!(storage.inner.get_parallel_merkle_latest_checkpoint().unwrap(), None);
        assert!(!storage.inner.has_parallel_merkle_checkpoint(7).unwrap());
        assert!(!storage.inner.has_parallel_merkle_checkpoint(9).unwrap());
    }
}
