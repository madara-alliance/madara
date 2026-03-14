use crate::{
    preconfirmed::PreconfirmedExecutedTransaction,
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
    storage::{DevnetPredeployedKeys, StorageHeadProjection, StoredChainInfo},
};
use mp_block::header::PreconfirmedHeader;
use mp_chain_config::{ChainConfig, RuntimeExecutionConfig, RuntimeExecutionConfigSerializable};
use rocksdb::{IteratorMode, ReadOptions};
use std::mem::size_of;

pub const META_COLUMN: Column = Column::new("meta").set_point_lookup();
pub const PRECONFIRMED_COLUMN: Column = Column::new("preconfirmed");

const META_DEVNET_KEYS_KEY: &[u8] = b"DEVNET_KEYS";
const META_LAST_SYNCED_L1_EVENT_BLOCK_KEY: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
const META_CONFIRMED_ON_L1_TIP_KEY: &[u8] = b"CONFIRMED_ON_L1_TIP";
const META_HEAD_PROJECTION_KEY: &[u8] = b"HEAD_PROJECTION";
// Legacy serialized metadata key used by older nodes before head-projection naming.
const META_HEAD_PROJECTION_LEGACY_KEY: &[u8] = &[67, 72, 65, 73, 78, 95, 84, 73, 80];
const META_CHAIN_INFO_KEY: &[u8] = b"CHAIN_INFO";
const META_LATEST_APPLIED_TRIE_UPDATE: &[u8] = b"LATEST_APPLIED_TRIE_UPDATE";
const META_RUNTIME_EXEC_CONFIG_KEY: &[u8] = b"RUNTIME_EXEC_CONFIG";
const META_SNAP_SYNC_LATEST_BLOCK: &[u8] = b"SNAP_SYNC_LATEST_BLOCK";
const META_PRECONFIRMED_HEADER_PREFIX: &[u8] = b"PRECONFIRMED_HEADER/";

// Parallel-merkle checkpoint metadata (kept in META_COLUMN).
const META_PARALLEL_MERKLE_CHECKPOINT_PREFIX: &[u8] = b"PARALLEL_MERKLE_CHECKPOINT/";
const META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY: &[u8] = b"PARALLEL_MERKLE_LATEST_CHECKPOINT";

fn meta_key_with_block_n(prefix: &[u8], block_n: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + size_of::<u64>());
    key.extend_from_slice(prefix);
    key.extend_from_slice(&block_n.to_be_bytes());
    key
}

fn preconfirmed_content_key(block_n: u64, tx_index: u16) -> [u8; 10] {
    let mut key = [0u8; 10];
    key[..8].copy_from_slice(&block_n.to_be_bytes());
    key[8..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

fn preconfirmed_block_range_start(block_n: u64) -> [u8; 10] {
    preconfirmed_content_key(block_n, 0)
}

fn preconfirmed_block_range_end_exclusive(block_n: u64) -> [u8; 10] {
    preconfirmed_content_key(block_n.saturating_add(1), 0)
}

fn preconfirmed_content_key_decode(key: &[u8]) -> Result<(u64, u16)> {
    anyhow::ensure!(key.len() == 10, "Malformed preconfirmed content key length: {}", key.len());
    let block_n = u64::from_be_bytes(key[0..8].try_into().context("Malformed preconfirmed block_n bytes")?);
    let tx_index = u16::from_be_bytes(key[8..10].try_into().context("Malformed preconfirmed tx_index bytes")?);
    Ok((block_n, tx_index))
}

fn preconfirmed_header_block_n_from_key(key: &[u8]) -> Result<u64> {
    anyhow::ensure!(key.starts_with(META_PRECONFIRMED_HEADER_PREFIX), "Malformed preconfirmed header key prefix");
    Ok(u64::from_be_bytes(
        key[META_PRECONFIRMED_HEADER_PREFIX.len()..].try_into().context("Malformed preconfirmed header key")?,
    ))
}

#[derive(serde::Deserialize, serde::Serialize)]
pub enum StoredHeadProjectionWithoutContent {
    Confirmed(u64),
    Preconfirmed(PreconfirmedHeader),
}

impl RocksDBStorageInner {
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
    pub(super) fn replace_head_projection(&self, head_projection: &StorageHeadProjection) -> Result<()> {
        // We need to do all of this in a single write. (atomic)

        let meta_col = self.get_column(META_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();
        // Keep metadata on the new key only.
        batch.delete_cf(&meta_col, META_HEAD_PROJECTION_LEGACY_KEY);

        // Write new head projection.
        match head_projection {
            StorageHeadProjection::Empty => batch.delete_cf(&meta_col, META_HEAD_PROJECTION_KEY),
            StorageHeadProjection::Confirmed(block_n) => {
                batch.put_cf(
                    &meta_col,
                    META_HEAD_PROJECTION_KEY,
                    super::serialize_to_smallvec::<[u8; 128]>(&StoredHeadProjectionWithoutContent::Confirmed(
                        *block_n,
                    ))?,
                );
            }
            StorageHeadProjection::Preconfirmed { header, content } => {
                batch.put_cf(
                    &meta_col,
                    META_HEAD_PROJECTION_KEY,
                    super::serialize_to_smallvec::<[u8; 128]>(&StoredHeadProjectionWithoutContent::Preconfirmed(
                        header.clone(),
                    ))?,
                );
                // Persist block-scoped header metadata. Content is appended separately.
                batch.put_cf(
                    &meta_col,
                    meta_key_with_block_n(META_PRECONFIRMED_HEADER_PREFIX, header.block_number),
                    super::serialize_to_smallvec::<[u8; 128]>(header)?,
                );
                // Keep the existing behavior of accepting content payloads from callers.
                // The key format is block-scoped: (block_n, tx_index).
                let block_n = header.block_number;
                for (tx_index, val) in content.iter().enumerate() {
                    let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
                    batch.put_cf(
                        &self.get_column(PRECONFIRMED_COLUMN),
                        preconfirmed_content_key(block_n, tx_index),
                        super::serialize(&val)?,
                    );
                }
            }
        };

        // Write head projection atomically
        // Note: Using regular write opts (no fsync) for performance
        // The head projection will be synced on next flush or graceful shutdown
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    // internal utility method
    #[tracing::instrument(skip(self))]
    pub(super) fn get_head_projection_without_content(&self) -> Result<Option<StoredHeadProjectionWithoutContent>> {
        let meta_col = self.get_column(META_COLUMN);
        if let Some(res) = self.db.get_pinned_cf(&meta_col, META_HEAD_PROJECTION_KEY)? {
            return Ok(Some(super::deserialize::<StoredHeadProjectionWithoutContent>(&res)?));
        }

        let Some(legacy) = self.db.get_pinned_cf(&meta_col, META_HEAD_PROJECTION_LEGACY_KEY)? else {
            return Ok(None);
        };

        // Lazy migration path for pre-existing databases.
        let mut batch = WriteBatchWithTransaction::default();
        batch.put_cf(&meta_col, META_HEAD_PROJECTION_KEY, legacy.as_ref());
        batch.delete_cf(&meta_col, META_HEAD_PROJECTION_LEGACY_KEY);
        self.db.write_opt(batch, &self.writeopts)?;

        Ok(Some(super::deserialize::<StoredHeadProjectionWithoutContent>(&legacy)?))
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
            batch.put_cf(&col, preconfirmed_content_key(block_n, tx_index), super::serialize(&value)?);
        }
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, header))]
    pub(super) fn write_preconfirmed_header(&self, header: &PreconfirmedHeader) -> Result<()> {
        self.db.put_cf_opt(
            &self.get_column(META_COLUMN),
            meta_key_with_block_n(META_PRECONFIRMED_HEADER_PREFIX, header.block_number),
            super::serialize_to_smallvec::<[u8; 128]>(header)?,
            &self.writeopts,
        )?;
        Ok(())
    }

    pub(super) fn get_preconfirmed_block_data(
        &self,
        block_n: u64,
    ) -> Result<Option<(PreconfirmedHeader, Vec<PreconfirmedExecutedTransaction>)>> {
        let meta_col = self.get_column(META_COLUMN);
        let header_key = meta_key_with_block_n(META_PRECONFIRMED_HEADER_PREFIX, block_n);
        let Some(header_raw) = self.db.get_pinned_cf(&meta_col, header_key)? else {
            return Ok(None);
        };
        let header: PreconfirmedHeader = super::deserialize(&header_raw)?;

        let mut content = Vec::new();
        let iter = DBIterator::new_cf(
            &self.db,
            &self.get_column(PRECONFIRMED_COLUMN),
            ReadOptions::default(),
            IteratorMode::From(&preconfirmed_block_range_start(block_n), rocksdb::Direction::Forward),
        )
        .into_iter_items(|(k, v)| -> Result<Option<PreconfirmedExecutedTransaction>> {
            let (row_block_n, _tx_index) = preconfirmed_content_key_decode(k)?;
            if row_block_n != block_n {
                return Ok(None);
            }
            Ok(Some(super::deserialize::<PreconfirmedExecutedTransaction>(v)?))
        });

        for item in iter {
            match item?? {
                Some(tx) => content.push(tx),
                None => break,
            }
        }

        Ok(Some((header, content)))
    }

    pub(super) fn get_latest_preconfirmed_header_block_n(&self) -> Result<Option<u64>> {
        let meta_col = self.get_column(META_COLUMN);
        let seek_key = meta_key_with_block_n(META_PRECONFIRMED_HEADER_PREFIX, u64::MAX);
        let iter = self.db.iterator_cf(&meta_col, IteratorMode::From(&seek_key, rocksdb::Direction::Reverse));
        for entry in iter {
            let (key, _value) = entry?;
            if !key.starts_with(META_PRECONFIRMED_HEADER_PREFIX) {
                break;
            }
            return Ok(Some(preconfirmed_header_block_n_from_key(&key)?));
        }
        Ok(None)
    }

    pub(super) fn delete_preconfirmed_rows_up_to(&self, confirmed_tip: u64) -> Result<()> {
        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);
        let meta_col = self.get_column(META_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();
        batch.delete_range_cf(
            &preconfirmed_col,
            preconfirmed_block_range_start(0),
            preconfirmed_block_range_end_exclusive(confirmed_tip),
        );

        let start = meta_key_with_block_n(META_PRECONFIRMED_HEADER_PREFIX, 0);
        for entry in self.db.iterator_cf(&meta_col, IteratorMode::From(&start, rocksdb::Direction::Forward)) {
            let (key, _value) = entry?;
            if !key.starts_with(META_PRECONFIRMED_HEADER_PREFIX) {
                break;
            }
            let block_n = preconfirmed_header_block_n_from_key(&key)?;
            if block_n <= confirmed_tip {
                batch.delete_cf(&meta_col, key);
            } else {
                break;
            }
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_head_projection(&self) -> Result<StorageHeadProjection> {
        match self.get_head_projection_without_content()? {
            None => Ok(StorageHeadProjection::Empty),
            Some(StoredHeadProjectionWithoutContent::Confirmed(block_n)) => {
                Ok(StorageHeadProjection::Confirmed(block_n))
            }
            Some(StoredHeadProjectionWithoutContent::Preconfirmed(header)) => {
                // Get preconfirmed block content
                let content = self
                    .get_preconfirmed_block_data(header.block_number)?
                    .map(|(_header, txs)| txs)
                    .unwrap_or_default();

                Ok(StorageHeadProjection::Preconfirmed { header, content })
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
        Ok(self
            .db
            .get_pinned_cf(
                &self.get_column(META_COLUMN),
                meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, block_n),
            )?
            .is_some())
    }

    pub(super) fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(META_COLUMN), META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY)?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    /// Find the latest checkpoint <= target_block_n.
    pub(super) fn get_parallel_merkle_checkpoint_floor(&self, target_block_n: u64) -> Result<Option<u64>> {
        let meta_col = self.get_column(META_COLUMN);
        let seek_key = meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, target_block_n);
        let iter = self.db.iterator_cf(&meta_col, IteratorMode::From(&seek_key, rocksdb::Direction::Reverse));
        for item in iter {
            let (key, _value) = item?;
            if !key.starts_with(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX) {
                break;
            }
            let block_n = u64::from_be_bytes(
                key[META_PARALLEL_MERKLE_CHECKPOINT_PREFIX.len()..]
                    .try_into()
                    .context("Malformed parallel merkle checkpoint key")?,
            );
            return Ok(Some(block_n));
        }
        Ok(None)
    }

    /// Remove checkpoint metadata entries for all blocks > target_block_n.
    pub(super) fn remove_parallel_merkle_checkpoints_above(&self, target_block_n: u64) -> Result<()> {
        let meta_col = self.get_column(META_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();

        if target_block_n == u64::MAX {
            return Ok(());
        }

        let seek_key = meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, target_block_n + 1);
        let iter = self.db.iterator_cf(&meta_col, IteratorMode::From(&seek_key, rocksdb::Direction::Forward));
        for item in iter {
            let (key, _value) = item?;
            if !key.starts_with(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX) {
                break;
            }
            let block_n = u64::from_be_bytes(
                key[META_PARALLEL_MERKLE_CHECKPOINT_PREFIX.len()..]
                    .try_into()
                    .context("Malformed parallel merkle checkpoint key")?,
            );
            if block_n > target_block_n {
                batch.delete_cf(&meta_col, key);
            }
        }

        let floor = self.get_parallel_merkle_checkpoint_floor(target_block_n)?;
        match floor {
            Some(checkpoint) => {
                batch.put_cf(
                    &meta_col,
                    META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY,
                    super::serialize_to_smallvec::<[u8; 16]>(&checkpoint)?,
                );
            }
            None => batch.delete_cf(&meta_col, META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY),
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
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
