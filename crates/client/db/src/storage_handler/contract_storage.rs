use std::sync::Arc;

use async_trait::async_trait;
use crossbeam_skiplist::{SkipMap, SkipSet};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use rocksdb::{IteratorMode, ReadOptions};
use starknet_api::core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_core::types::{ContractStorageDiffItem, StateDiff, StorageEntry};
use tokio::task::{spawn_blocking, JoinSet};

use super::history::History;
use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractStorageViewMut(SkipMap<(ContractAddress, StorageKey), StarkFelt>);
pub struct ContractStorageView;

#[async_trait]
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
    #[must_use]
    #[allow(clippy::type_complexity, clippy::type_repetition_in_bounds)]
    async fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = Arc::new(DeoxysBackend::expose_db());

        let mut set = JoinSet::new();
        for (key, value) in self.0.into_iter() {
            let db = Arc::clone(&db);
            set.spawn(async move {
                let column = db.get_column(Column::ContractStorage);

                let mut history: History<StarkFelt> = match db.get_cf(&column, bincode::serialize(&key).unwrap()) {
                    Ok(Some(bytes)) => match bincode::deserialize(&bytes) {
                        Ok(history) => history,
                        Err(_) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
                    },
                    Ok(None) => History::default(),
                    Err(_) => return Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage)),
                };
                if history.push(block_number, value).is_err() {
                    return Err(DeoxysStorageError::StorageCommitError(StorageType::ContractStorage));
                }

                db.put_cf(&column, bincode::serialize(&key).unwrap(), bincode::serialize(&history).unwrap())
                    .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractStorage))
            });
        }

        while let Some(res) = set.join_next().await {
            res.unwrap()?;
        }

        Ok(())
    }
}

impl StorageView for ContractStorageViewMut {
    type KEY = (ContractAddress, StorageKey);
    type VALUE = StarkFelt;

    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        if let Some(value) = self.0.get(key).map(|entry| entry.value().clone()) {
            return Ok(Some(value));
        }

        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        let history: History<StarkFelt> = match db
            .get_cf(&column, bincode::serialize(key).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| bincode::deserialize(&bytes))
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

        match db.key_may_exist_cf(&column, bincode::serialize(&key).unwrap()) {
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
        read_options.set_iterate_lower_bound(bincode::serialize(&block_number).unwrap());

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
        let keys = [block_number..block_number_max].into_iter().map(|key| bincode::serialize(&key).unwrap());
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
            let key = bincode::serialize(&key).unwrap();

            set.spawn(async move {
                let column = db.get_column(Column::ContractStorage);
                let Ok(result) = db.get_cf(&column, key.clone()) else {
                    return Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage));
                };
                let mut history: History<StarkFelt> = match result.map(|bytes| bincode::deserialize(&bytes)) {
                    Some(Ok(history)) => history,
                    Some(Err(_)) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
                    None => unreachable!("Reverting contract storage should only use existing contract addresses"),
                };

                // history is updated and re-inserted into the db
                // or deleted if reverting leaves it empty
                history.revert_to(block_number);
                match history.is_empty() {
                    true => {
                        if db.delete(key.clone()).is_err() {
                            return Err(DeoxysStorageError::StorageRevertError(
                                StorageType::ContractStorage,
                                block_number,
                            ));
                        }
                    }
                    false => {
                        if db.put_cf(&column, key.clone(), bincode::serialize(&history).unwrap()).is_err() {
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

impl ContractStorageViewMut {
    pub fn get_at(
        &self,
        key: &(ContractAddress, StorageKey),
        block_number: u64,
    ) -> Result<Option<StarkFelt>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractStorage);

        let history: History<StarkFelt> = match db
            .get_cf(&column, bincode::serialize(key).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| bincode::deserialize(&bytes))
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
            .get_cf(&column, bincode::serialize(key).unwrap())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .map(|bytes| bincode::deserialize(&bytes))
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

        match db.key_may_exist_cf(&column, bincode::serialize(&key).unwrap()) {
            true => Ok(self.get(key)?.is_some()),
            false => Ok(false),
        }
    }
}
