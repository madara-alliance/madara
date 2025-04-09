#![doc = include_str!("../docs/flat_storage.md")]

use std::{collections::HashSet, sync::Arc};

use mp_state_update::StateDiff;
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
use rocksdb::{BoundColumnFamily, IteratorMode, ReadOptions, WriteOptions};
use serde::Serialize;
use starknet_types_core::felt::Felt;

use crate::{
    db_block_id::{DbBlockId, DbBlockIdResolvable},
    Column, DatabaseExt, MadaraBackend, MadaraStorageError, WriteBatchWithTransaction, DB, DB_UPDATES_BATCH_SIZE,
};

// NB: Columns cf needs prefix extractor of these length during creation
pub(crate) const CONTRACT_STORAGE_PREFIX_EXTRACTOR: usize = 64;
pub(crate) const CONTRACT_CLASS_HASH_PREFIX_EXTRACTOR: usize = 32;
pub(crate) const CONTRACT_NONCES_PREFIX_EXTRACTOR: usize = 32;

const LAST_KEY: &[u8] = &[0xFF; 64];

fn make_storage_key_prefix(contract_address: Felt, storage_key: Felt) -> [u8; 64] {
    let mut key = [0u8; 64];
    key[..32].copy_from_slice(contract_address.to_bytes_be().as_ref());
    key[32..].copy_from_slice(storage_key.to_bytes_be().as_ref());
    key
}

impl MadaraBackend {
    #[tracing::instrument(skip(self, id, k, make_bin_prefix), fields(module = "ContractDB"))]
    fn resolve_history_kv<K: serde::Serialize, V: serde::de::DeserializeOwned, B: AsRef<[u8]>>(
        &self,
        id: &impl DbBlockIdResolvable,
        pending_col: Column,
        nonpending_col: Column,
        k: &K,
        make_bin_prefix: impl FnOnce(&K) -> B,
    ) -> Result<Option<V>, MadaraStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        let block_n = match id {
            DbBlockId::Pending => {
                // Get pending or fallback to latest block_n
                let col = self.db.get_column(pending_col);
                // todo: smallint here to avoid alloc

                // Note: pending has keys in bincode, not bytes
                if let Some(res) = self.db.get_pinned_cf(&col, bincode::serialize(k)?)? {
                    return Ok(Some(bincode::deserialize(&res)?)); // found in pending
                }

                let Some(block_n) = self.get_latest_block_n()? else { return Ok(None) };
                block_n
            }
            DbBlockId::Number(block_n) => block_n,
        };

        // We try to find history values.

        let block_n = u32::try_from(block_n).map_err(|_| MadaraStorageError::InvalidBlockNumber)?;
        let bin_prefix = make_bin_prefix(k);
        let start_at = [bin_prefix.as_ref(), &block_n.to_be_bytes() as &[u8]].concat();

        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        // We don't need ot set an iteration range as we have set up a prefix extractor for the column.
        // We are doing prefix iteration
        // options.set_iterate_range(PrefixRange(&prefix as &[u8]));
        let mode = IteratorMode::From(&start_at, rocksdb::Direction::Reverse);
        // TODO(perf): It is possible to iterate in a pinned way, using raw iter
        let mut iter = self.db.iterator_cf_opt(&self.db.get_column(nonpending_col), options, mode);

        match iter.next() {
            Some(res) => {
                #[allow(unused_variables)]
                let (k, v) = res?;
                #[cfg(debug_assertions)]
                assert!(k.starts_with(bin_prefix.as_ref())); // This should fail if we forgot to set up a prefix iterator for the column.

                Ok(Some(bincode::deserialize(&v)?))
            }
            None => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "ContractDB"))]
    pub fn is_contract_deployed_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract_addr: &Felt,
    ) -> Result<bool, MadaraStorageError> {
        // TODO(perf): use rocksdb key_may_exists bloom filters
        Ok(self.get_contract_class_hash_at(id, contract_addr)?.is_some())
    }

    #[tracing::instrument(skip(self, id, contract_addr), fields(module = "ContractDB"))]
    pub fn get_contract_class_hash_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract_addr: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        self.resolve_history_kv(
            id,
            Column::PendingContractToClassHashes,
            Column::ContractToClassHashes,
            contract_addr,
            |k| k.to_bytes_be(),
        )
    }

    #[tracing::instrument(skip(self, id), fields(module = "ContractDB"))]
    pub fn get_contract_nonce_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract_addr: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        self.resolve_history_kv(id, Column::PendingContractToNonces, Column::ContractToNonces, contract_addr, |k| {
            k.to_bytes_be()
        })
    }

    #[tracing::instrument(skip(self, id, key), fields(module = "ContractDB"))]
    pub fn get_contract_storage_at(
        &self,
        id: &impl DbBlockIdResolvable,
        contract_addr: &Felt,
        key: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        self.resolve_history_kv(
            id,
            Column::PendingContractStorage,
            Column::ContractStorage,
            &(*contract_addr, *key),
            |(k1, k2)| make_storage_key_prefix(*k1, *k2),
        )
    }

    /// NB: This functions needs to run on the rayon thread pool
    #[tracing::instrument(
        skip(self, block_number, contract_class_updates, contract_nonces_updates, contract_kv_updates),
        fields(module = "ContractDB")
    )]
    pub(crate) fn contract_db_store_block(
        &self,
        block_number: u64,
        contract_class_updates: &[(Felt, Felt)],
        contract_nonces_updates: &[(Felt, Felt)],
        contract_kv_updates: &[((Felt, Felt), Felt)],
    ) -> Result<(), MadaraStorageError> {
        let block_number = u32::try_from(block_number).map_err(|_| MadaraStorageError::InvalidBlockNumber)?;

        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        fn write_chunk(
            db: &DB,
            writeopts: &WriteOptions,
            col: &Arc<BoundColumnFamily>,
            block_number: u32,
            chunk: impl IntoIterator<Item = (impl AsRef<[u8]>, Felt)>,
        ) -> Result<(), MadaraStorageError> {
            let mut batch = WriteBatchWithTransaction::default();
            for (key, value) in chunk {
                // TODO: find a way to avoid this allocation
                let key = [key.as_ref(), &block_number.to_be_bytes() as &[u8]].concat();
                batch.put_cf(col, key, bincode::serialize(&value)?);
            }
            db.write_opt(batch, writeopts)?;
            Ok(())
        }

        contract_class_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::ContractToClassHashes),
            |col, chunk| {
                write_chunk(&self.db, &writeopts, col, block_number, chunk.iter().map(|(k, v)| (k.to_bytes_be(), *v)))
            },
        )?;
        contract_nonces_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::ContractToNonces),
            |col, chunk| {
                write_chunk(&self.db, &writeopts, col, block_number, chunk.iter().map(|(k, v)| (k.to_bytes_be(), *v)))
            },
        )?;
        contract_kv_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::ContractStorage),
            |col, chunk| {
                write_chunk(
                    &self.db,
                    &writeopts,
                    col,
                    block_number,
                    chunk.iter().map(|((k1, k2), v)| {
                        let mut key = [0u8; 64];
                        key[..32].copy_from_slice(k1.to_bytes_be().as_ref());
                        key[32..].copy_from_slice(k2.to_bytes_be().as_ref());
                        (key, *v)
                    }),
                )
            },
        )?;

        Ok(())
    }

    // TODO: does this need rayon thread pool? what about #[tracing::instrument()] ?
    pub(crate) fn contract_db_revert(
        &self,
        revert_to: u64,
        state_diffs: &Vec<StateDiff>,
    ) -> Result<(), MadaraStorageError> {
        let contract_to_class_hashes_col = self.db.get_column(Column::ContractToClassHashes);
        let contract_to_nonces_col = self.db.get_column(Column::ContractToNonces);
        let contract_storage_col = self.db.get_column(Column::ContractStorage);

        let mut contract_to_class_hashes_keys = HashSet::new();
        let mut contract_to_nonce_keys = HashSet::new();
        let mut contract_storage_keys = HashSet::new();

        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);
        let mut batch = WriteBatchWithTransaction::default();

        // For each block, we want to delete all contract storage for the given block.
        //
        // The database stores them with a compound key that includes the block number,
        // so the previous state is implicitly present (and becomes the latest) after
        // deleting for reverted blocks.
        //
        // This also allows us to not care about the actual changes in the state diff,
        // we only need to care about which keys to prune.
        for diff in state_diffs {
            diff.deployed_contracts.iter().for_each(|item| {
                contract_to_class_hashes_keys.insert(item.address);
            });
            diff.replaced_classes.iter().for_each(|item| {
                contract_to_class_hashes_keys.insert(item.contract_address);
            });

            diff.nonces.iter().for_each(|update| {
                contract_to_nonce_keys.insert(update.contract_address);
            });

            // contract storage is a compound key (contract_address:storage_address)
            diff.storage_diffs.iter().for_each(|diff_item| {
                diff_item.storage_entries.iter().for_each(|entry| {
                    contract_storage_keys.insert((diff_item.address, entry.key));
                });
            });
        }
        let latest_block_n = self.get_latest_block_n()?.unwrap(); // TODO: unwrap - Option here probably relates to genesis block
        for block_n in (revert_to + 1..latest_block_n + 1).rev() {
            // for each entry in the keys we collected above, create a db key with that entry and the block number and delete it
            // TODO: we may be able to leverage RocksDb's tree-based iterator here to avoid looping over each block

            for contract_address in &contract_to_class_hashes_keys {
                let contract_key = [&contract_address.to_bytes_be()[..], &block_n.to_be_bytes() as &[u8]].concat();
                batch.delete_cf(&contract_to_class_hashes_col, contract_key);
            }

            for contract_address in &contract_to_nonce_keys {
                let contract_key = [&contract_address.to_bytes_be()[..], &block_n.to_be_bytes() as &[u8]].concat();
                batch.delete_cf(&contract_to_nonces_col, contract_key);
            }

            for (contract_address, storage_key) in &contract_storage_keys {
                let contract_key = [
                    &contract_address.to_bytes_be()[..],
                    &storage_key.to_bytes_be()[..],
                    &block_n.to_be_bytes() as &[u8],
                ]
                .concat();
                batch.delete_cf(&contract_storage_col, contract_key);
            }
        }

        self.db.write_opt(batch, &writeopts)?;

        Ok(())
    }

    /// NB: This functions needs to run on the rayon thread pool
    #[tracing::instrument(
        skip(self, contract_class_updates, contract_nonces_updates, contract_kv_updates),
        fields(module = "ContractDB")
    )]
    pub(crate) fn contract_db_store_pending(
        &self,
        contract_class_updates: &[(Felt, Felt)],
        contract_nonces_updates: &[(Felt, Felt)],
        contract_kv_updates: &[((Felt, Felt), Felt)],
    ) -> Result<(), MadaraStorageError> {
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        // Note: pending has keys in bincode, not bytes

        fn write_chunk(
            db: &DB,
            writeopts: &WriteOptions,
            col: &Arc<BoundColumnFamily>,
            chunk: impl IntoIterator<Item = (impl Serialize, Felt)>,
        ) -> Result<(), MadaraStorageError> {
            let mut batch = WriteBatchWithTransaction::default();
            for (key, value) in chunk {
                // TODO: find a way to avoid this allocation
                batch.put_cf(col, bincode::serialize(&key)?, bincode::serialize(&value)?);
            }
            db.write_opt(batch, writeopts)?;
            Ok(())
        }

        contract_class_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::PendingContractToClassHashes),
            |col, chunk| write_chunk(&self.db, &writeopts, col, chunk.iter().map(|(k, v)| (k, *v))),
        )?;
        contract_nonces_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::PendingContractToNonces),
            |col, chunk| write_chunk(&self.db, &writeopts, col, chunk.iter().map(|(k, v)| (k, *v))),
        )?;
        contract_kv_updates.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(Column::PendingContractStorage),
            |col, chunk| write_chunk(&self.db, &writeopts, col, chunk.iter().map(|((k1, k2), v)| ((k1, k2), *v))),
        )?;

        Ok(())
    }

    #[tracing::instrument(fields(module = "ContractDB"))]
    pub(crate) fn contract_db_clear_pending(&self) -> Result<(), MadaraStorageError> {
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        self.db.delete_range_cf_opt(
            &self.db.get_column(Column::PendingContractToNonces),
            &[] as _,
            LAST_KEY,
            &writeopts,
        )?;
        self.db.delete_range_cf_opt(
            &self.db.get_column(Column::PendingContractToClassHashes),
            &[] as _,
            LAST_KEY,
            &writeopts,
        )?;
        self.db.delete_range_cf_opt(
            &self.db.get_column(Column::PendingContractStorage),
            &[] as _,
            LAST_KEY,
            &writeopts,
        )?;

        Ok(())
    }
}
