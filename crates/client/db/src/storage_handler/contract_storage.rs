use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use crossbeam_skiplist::{SkipMap, SkipSet};
use mp_convert::field_element::FromFieldElement;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use rayon::slice::ParallelSlice;
use rocksdb::{IteratorMode, ReadOptions, WriteBatchWithTransaction};
use starknet_api::core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_core::types::{ContractStorageDiffItem, StateDiff, StorageEntry};
use tokio::task::{spawn_blocking, JoinSet};

use super::codec::{self, Decode};
use super::history::History;
use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::storage_handler::codec::Encode;
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Clone)]
struct ContractStorageKeyEncoded(pub [u8; 64]);
impl ContractStorageKeyEncoded {
    fn new(address: &ContractAddress, key: &StorageKey) -> Self {
        let mut arr = [0u8; 64];
        arr[..32].copy_from_slice(address.bytes());
        arr[32..].copy_from_slice(key.bytes());
        Self(arr)
    }
    // fn contract_address(&self) -> Option<ContractAddress> {
    //     // Safety: this returns a slice of 32 bytes
    //     let bytes = self.0[..32].try_into().unwrap();
    //     Some(ContractAddress(PatriciaKey(StarkFelt::new(bytes).ok()?)))
    // }
    // fn storage_key(&self) -> Option<StorageKey> {
    //     // Safety: this returns a slice of 32 bytes
    //     let bytes = self.0[32..].try_into().unwrap();
    //     Some(StorageKey(PatriciaKey(StarkFelt::new(bytes).ok()?)))
    // }
}
impl AsRef<[u8]> for ContractStorageKeyEncoded {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Default, Debug)]
pub struct ContractStorageViewMut(SkipMap<(ContractAddress, StorageKey), StarkFelt>);
pub struct ContractStorageView;

impl StorageViewMut for ContractStorageViewMut {
    type KEY = (ContractAddress, StorageKey);

    type VALUE = StarkFelt;

    /// Insert data into storage.
    ///
    /// * `key`: identifier used to inser data.
    /// * `value`: encodable data to save to the database.
    fn insert(&self, key: Self::KEY, value: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(key, value);
        Ok(())
    }

    /// Applies all changes up to this point.
    ///
    /// * `block_number`: point in the chain at which to apply the new changes. Must be
    /// incremental
    fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = Arc::new(DeoxysBackend::expose_db());

        let as_vec = self.0.into_iter().collect::<Vec<_>>(); // todo: use proper datastructure that supports rayon

        as_vec.deref().par_chunks(1024).try_for_each(|chunk| {
            let column = db.get_column(Column::ContractStorage);
            let histories_encoded = db
                .multi_get_cf(chunk.iter().map(|(key, _v)| (&column, ContractStorageKeyEncoded::new(&key.0, &key.1))));

            let mut batch = WriteBatchWithTransaction::<true>::default();
            for (history_encoded, (key, value)) in histories_encoded.into_iter().zip(chunk) {
                let key_encoded = ContractStorageKeyEncoded::new(&key.0, &key.1);

                let mut history_encoded = history_encoded
                    .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
                    .unwrap_or_default();

                codec::add_to_history_encoded(&mut history_encoded, block_number, *value)?;
                batch.put_cf(&column, key_encoded, history_encoded);
            }

            db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractStorage))?;
            Ok::<_, DeoxysStorageError>(())
        })?;

        Ok(())
    }
}

impl StorageView for ContractStorageViewMut {
    type KEY = (ContractAddress, StorageKey);
    type VALUE = StarkFelt;

    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        if let Some(value) = self.0.get(key).map(|entry| *entry.value()) {
            return Ok(Some(value));
        }

        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        let history: History<StarkFelt> = match db
            .get_cf(&column, ContractStorageKeyEncoded::new(&key.0, &key.1))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| Decode::decode(&bytes))
        {
            Some(Ok(history)) => history,
            Some(Err(_)) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
            None => History::default(),
        };

        Ok(history.get().copied())
    }

    fn contains(&self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        match db.key_may_exist_cf(&column, ContractStorageKeyEncoded::new(&key.0, &key.1)) {
            true => Ok(self.get(key)?.is_some()),
            false => Ok(false),
        }
    }
}

#[async_trait]
impl StorageViewRevetible for ContractStorageViewMut {
    async fn revert_to(&self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = Arc::new(DeoxysBackend::expose_db());

        let mut read_options = ReadOptions::default();
        read_options.set_iterate_lower_bound(bincode::serialize(&block_number)?);

        // Currently we only use this iterator to retrieve the latest block number. A better way to
        // do this would be to decouple the highest block lazy_static in `l2.rs`, but in the
        // meantime this works as a workaround.
        let mut iter = db.iterator_cf_opt(&db.get_column(Column::BlockStateDiff), read_options, IteratorMode::End);
        let block_number_max = match iter.next() {
            Some(Ok((bytes, _))) => bincode::deserialize(&bytes)
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage))?,
            Some(Err(_)) => return Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage)),
            None => 0,
        };
        assert!(block_number <= block_number_max);

        // First, we aggregate all storage changes from the latest [StateDiff]
        // up to the target block number. We use a non-blocking set and perform this asychronously.
        // TODO: buffer this
        let keys = (block_number..block_number_max).map(|key| bincode::serialize(&key).unwrap());
        let change_set = Arc::new(SkipSet::new());
        for entry in db.batched_multi_get_cf(&db.get_column(Column::BlockStateDiff), keys, true) {
            let state_diff = match entry {
                Ok(Some(bytes)) => bincode::deserialize::<StateDiff>(&bytes)
                    .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::BlockStateDiff))?,
                _ => return Err(DeoxysStorageError::StorageRetrievalError(StorageType::BlockStateDiff)),
            };

            let change_set = Arc::clone(&change_set);
            spawn_blocking(move || {
                state_diff.storage_diffs.into_par_iter().for_each(
                    |ContractStorageDiffItem { address, storage_entries }| {
                        storage_entries.into_par_iter().for_each(|StorageEntry { key, value: _ }| {
                            change_set.insert((address, key));
                        })
                    },
                );
            });
        }

        // Next, we iterate over all contract storage keys that have changed,
        // reverting them to their state at the given block number
        let mut set = JoinSet::new();
        let change_set = Arc::try_unwrap(change_set).expect("Arc should not be aliased");
        for key in change_set.into_iter() {
            let db = Arc::clone(&db);
            let key = (ContractAddress::from_field_element(key.0), StorageKey::from_field_element(key.1));
            let key_encoded = ContractStorageKeyEncoded::new(&key.0, &key.1);

            set.spawn(async move {
                let column = db.get_column(Column::ContractStorage);
                let Ok(result) = db.get_cf(&column, key_encoded.clone()) else {
                    return Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage));
                };
                let mut history: History<StarkFelt> = match result.map(|bytes| Decode::decode(&bytes)) {
                    Some(Ok(history)) => history,
                    Some(Err(_)) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
                    None => unreachable!("Reverting contract storage should only use existing contract addresses"),
                };

                // history is updated and re-inserted into the db
                // or deleted if reverting leaves it empty
                history.revert_to(block_number);
                match history.is_empty() {
                    true => {
                        if db.delete(key_encoded.clone()).is_err() {
                            return Err(DeoxysStorageError::StorageRevertError(
                                StorageType::ContractStorage,
                                block_number,
                            ));
                        }
                    }
                    false => {
                        if db.put_cf(&column, key_encoded.clone(), history.encode()?).is_err() {
                            return Err(DeoxysStorageError::StorageRevertError(
                                StorageType::ContractStorage,
                                block_number,
                            ));
                        }
                    }
                }

                Ok(())
            });
        }

        while let Some(res) = set.join_next().await {
            res.unwrap()?;
        }

        Ok(())
    }
}

impl ContractStorageView {
    pub fn get_at(
        &self,
        key: &(ContractAddress, StorageKey),
        block_number: u64,
    ) -> Result<Option<StarkFelt>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        let history: History<StarkFelt> = match db
            .get_cf(&column, ContractStorageKeyEncoded::new(&key.0, &key.1))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| Decode::decode(&bytes))
        {
            Some(Ok(history)) => history,
            Some(Err(_)) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
            None => History::default(),
        };

        Ok(history.get_at(block_number).copied())
    }
}

impl StorageView for ContractStorageView {
    type KEY = (ContractAddress, StorageKey);
    type VALUE = StarkFelt;

    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        let history: History<StarkFelt> = match db
            .get_cf(&column, ContractStorageKeyEncoded::new(&key.0, &key.1))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| Decode::decode(&bytes))
        {
            Some(Ok(history)) => history,
            Some(Err(_)) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
            None => History::default(),
        };

        Ok(history.get().copied())
    }

    fn contains(&self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        match db.key_may_exist_cf(&column, ContractStorageKeyEncoded::new(&key.0, &key.1)) {
            true => Ok(self.get(key)?.is_some()),
            false => Ok(false),
        }
    }
}
