#![doc = include_str!("../docs/flat_storage.md")]

use rayon::{
    iter::{IntoParallelRefIterator, ParallelIterator},
    slice::ParallelSlice,
};

use crate::DatabaseExt;

type ContractAddress = starknet_types_core::felt::Felt;
type ClassHash = starknet_types_core::felt::Felt;
type Nonce = starknet_types_core::felt::Felt;
type StorageKey = (ContractAddress, starknet_types_core::felt::Felt);
type StorageVal = starknet_types_core::felt::Felt;

#[derive(Default, Debug)]
pub struct ContractUpdates {
    contract_address_to_class_hash: std::collections::BTreeMap<ContractAddress, ClassHash>,
    contract_address_to_nonce: std::collections::BTreeMap<ContractAddress, Nonce>,
    contract_address_to_storage: std::collections::BTreeMap<StorageKey, StorageVal>,
}

impl ContractUpdates {
    pub fn from_state_diff(state_diff: mp_state_update::StateDiff) -> Self {
        let contract_address_to_class_hash = state_diff
            .replaced_classes
            .into_iter()
            .map(|mp_state_update::ReplacedClassItem { contract_address, class_hash }| (contract_address, class_hash))
            .chain(
                state_diff
                    .deployed_contracts
                    .into_iter()
                    .map(|mp_state_update::DeployedContractItem { address, class_hash }| (address, class_hash)),
            )
            .collect();

        let contract_address_to_nonce = state_diff
            .nonces
            .into_iter()
            .map(|mp_state_update::NonceUpdate { contract_address, nonce }| (contract_address, nonce))
            .collect();

        let contract_address_to_storage = state_diff
            .storage_diffs
            .into_iter()
            .flat_map(|mp_state_update::ContractStorageDiffItem { address, storage_entries }| {
                storage_entries
                    .into_iter()
                    .map(move |mp_state_update::StorageEntry { key, value }| ((address, key), value))
            })
            .collect();

        Self { contract_address_to_class_hash, contract_address_to_nonce, contract_address_to_storage }
    }
}

// NB: Columns cf needs prefix extractor of these length during creation
pub(crate) const CONTRACT_STORAGE_PREFIX_LEN: usize = 64;
pub(crate) const CONTRACT_CLASS_HASH_PREFIX_LEN: usize = 32;
pub(crate) const CONTRACT_NONCES_PREFIX_LEN: usize = 32;

fn make_storage_key_prefix(
    contract_address: starknet_types_core::felt::Felt,
    storage_key: starknet_types_core::felt::Felt,
) -> [u8; 64] {
    let mut key = [0u8; 64];
    key[..32].copy_from_slice(contract_address.to_bytes_be().as_ref());
    key[32..].copy_from_slice(storage_key.to_bytes_be().as_ref());
    key
}

impl crate::MadaraBackend {
    #[tracing::instrument(skip(self, block_n, k, make_bin_prefix), fields(module = "ContractDB"))]
    fn resolve_history_kv<K: serde::Serialize, V: serde::de::DeserializeOwned, B: AsRef<[u8]>>(
        &self,
        block_n: u64,
        col: crate::Column,
        k: &K,
        make_bin_prefix: impl FnOnce(&K) -> B,
    ) -> Result<Option<V>, crate::MadaraStorageError> {
        let block_n = u32::try_from(block_n).map_err(|_| crate::MadaraStorageError::InvalidBlockNumber)?;
        let bin_prefix = make_bin_prefix(k);
        let start_at = [bin_prefix.as_ref(), &block_n.to_be_bytes() as &[u8]].concat();

        let mut options = rocksdb::ReadOptions::default();
        options.set_prefix_same_as_start(true);

        // We don't need to set an iteration range as we have set up a prefix extractor for the column.
        // We are doing prefix iteration
        // options.set_iterate_range(PrefixRange(&prefix as &[u8]));
        let mode = rocksdb::IteratorMode::From(&start_at, rocksdb::Direction::Reverse);

        // TODO(perf): It is possible to iterate in a pinned way, using raw iter
        let mut iter = self.db.iterator_cf_opt(&self.db.get_column(col), options, mode);

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
        id: &impl crate::db_block_id::DbBlockIdResolvable,
        contract_addr: &starknet_types_core::felt::Felt,
    ) -> Result<bool, crate::MadaraStorageError> {
        // TODO(perf): use rocksdb key_may_exists bloom filters
        Ok(self.get_contract_class_hash_at(id, contract_addr)?.is_some())
    }

    #[tracing::instrument(skip(self, id), fields(module = "ContractDB"))]
    pub fn get_contract_class_hash_at(
        &self,
        id: &impl crate::db_block_id::DbBlockIdResolvable,
        contract_address: &starknet_types_core::felt::Felt,
    ) -> Result<Option<starknet_types_core::felt::Felt>, crate::MadaraStorageError> {
        match id.resolve_db_block_id(self)? {
            Some(crate::db_block_id::RawDbBlockId::Pending) => {
                Ok(self.pending_latest().contracts.contract_address_to_class_hash.get(contract_address).cloned())
            }
            Some(crate::db_block_id::RawDbBlockId::Number(block_n)) => {
                self.resolve_history_kv(block_n, crate::Column::ContractToClassHashes, contract_address, |k| {
                    k.to_bytes_be()
                })
            }
            None => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id), fields(module = "ContractDB"))]
    pub fn get_contract_nonce_at(
        &self,
        id: &impl crate::db_block_id::DbBlockIdResolvable,
        contract_address: &starknet_types_core::felt::Felt,
    ) -> Result<Option<starknet_types_core::felt::Felt>, crate::MadaraStorageError> {
        match id.resolve_db_block_id(self)? {
            Some(crate::db_block_id::RawDbBlockId::Pending) => {
                Ok(self.pending_latest().contracts.contract_address_to_nonce.get(contract_address).cloned())
            }
            Some(crate::db_block_id::RawDbBlockId::Number(block_n)) => {
                self.resolve_history_kv(block_n, crate::Column::ContractToNonces, contract_address, |k| k.to_bytes_be())
            }
            None => Ok(None),
        }
    }

    #[tracing::instrument(skip(self, id, key), fields(module = "ContractDB"))]
    pub fn get_contract_storage_at(
        &self,
        id: &impl crate::db_block_id::DbBlockIdResolvable,
        contract_address: &starknet_types_core::felt::Felt,
        key: &starknet_types_core::felt::Felt,
    ) -> Result<Option<starknet_types_core::felt::Felt>, crate::MadaraStorageError> {
        let storage_key = (*contract_address, *key);
        match id.resolve_db_block_id(self)? {
            Some(crate::db_block_id::RawDbBlockId::Pending) => {
                Ok(self.pending_latest().contracts.contract_address_to_storage.get(&storage_key).cloned())
            }
            Some(crate::db_block_id::RawDbBlockId::Number(block_n)) => {
                self.resolve_history_kv(block_n, crate::Column::ContractStorage, &storage_key, |(k1, k2)| {
                    make_storage_key_prefix(*k1, *k2)
                })
            }
            None => Ok(None),
        }
    }

    /// NB: This functions needs to run on the rayon thread pool
    #[tracing::instrument(skip(self, block_number, value), fields(module = "ContractDB"))]
    pub(crate) fn contract_db_store_block(
        &self,
        block_number: u64,
        value: ContractUpdates,
    ) -> Result<(), crate::MadaraStorageError> {
        let block_number = u32::try_from(block_number).map_err(|_| crate::MadaraStorageError::InvalidBlockNumber)?;

        value
            .contract_address_to_class_hash
            .par_iter()
            .collect::<Vec<_>>()
            .par_chunks(crate::DB_UPDATES_BATCH_SIZE)
            .try_for_each_init(
                || self.db.get_column(crate::Column::ContractToClassHashes),
                |col, chunk| {
                    let mut batch = crate::WriteBatchWithTransaction::default();
                    self.contract_db_store_chunk(
                        col,
                        block_number,
                        chunk.iter().map(|(k, v)| (k.to_bytes_be(), **v)),
                        &mut batch,
                    )?;
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    Result::<(), crate::MadaraStorageError>::Ok(())
                },
            )?;
        value
            .contract_address_to_nonce
            .par_iter()
            .collect::<Vec<_>>()
            .par_chunks(crate::DB_UPDATES_BATCH_SIZE)
            .try_for_each_init(
                || self.db.get_column(crate::Column::ContractToNonces),
                |col, chunk| {
                    let mut batch = crate::WriteBatchWithTransaction::default();
                    self.contract_db_store_chunk(
                        col,
                        block_number,
                        chunk.iter().map(|(k, v)| (k.to_bytes_be(), **v)),
                        &mut batch,
                    )?;
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    Result::<(), crate::MadaraStorageError>::Ok(())
                },
            )?;
        value
            .contract_address_to_storage
            .par_iter()
            .collect::<Vec<_>>()
            .par_chunks(crate::DB_UPDATES_BATCH_SIZE)
            .try_for_each_init(
                || self.db.get_column(crate::Column::ContractStorage),
                |col, chunk| {
                    let mut batch = crate::WriteBatchWithTransaction::default();
                    self.contract_db_store_chunk(
                        col,
                        block_number,
                        chunk.iter().map(|((k1, k2), v)| {
                            let mut key = [0u8; 64];
                            key[..32].copy_from_slice(k1.to_bytes_be().as_ref());
                            key[32..].copy_from_slice(k2.to_bytes_be().as_ref());
                            (key, **v)
                        }),
                        &mut batch,
                    )?;
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    Result::<(), crate::MadaraStorageError>::Ok(())
                },
            )?;

        Ok(())
    }

    fn contract_db_store_chunk(
        &self,
        col: &std::sync::Arc<rocksdb::BoundColumnFamily>,
        block_number: u32,
        chunk: impl IntoIterator<Item = (impl AsRef<[u8]>, starknet_types_core::felt::Felt)>,
        tx: &mut crate::WriteBatchWithTransaction,
    ) -> Result<(), crate::MadaraStorageError> {
        for (key, value) in chunk {
            // TODO: find a way to avoid this allocation
            let key = [key.as_ref(), &block_number.to_be_bytes() as &[u8]].concat();
            tx.put_cf(col, key, bincode::serialize(&value)?);
        }
        Ok(())
    }
}
