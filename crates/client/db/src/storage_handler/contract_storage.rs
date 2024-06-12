use std::ops::Deref;

use starknet_api::core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

use super::history::{AsHistoryView, HistoryView, HistoryViewMut};
use crate::Column;

// NB: Column::ContractStorage cf needs prefix extractor of this length during creation
pub(crate) const CONTRACT_STORAGE_PREFIX_EXTRACTOR: usize = 64;

pub struct ContractAddressStorageKey([u8; 64]);
impl From<(ContractAddress, StorageKey)> for ContractAddressStorageKey {
    fn from(value: (ContractAddress, StorageKey)) -> Self {
        let mut key = [0u8; 64];
        key[..32].copy_from_slice(value.0 .0.key().bytes());
        key[32..].copy_from_slice(value.1 .0.key().bytes());
        Self(key)
    }
}
impl Deref for ContractAddressStorageKey {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct ContractStorageAsHistory;
impl AsHistoryView for ContractStorageAsHistory {
    type Key = (ContractAddress, StorageKey);
    type KeyBin = ContractAddressStorageKey;
    type T = StarkFelt;
    fn column() -> Column {
        Column::ContractStorage
    }
}

pub type ContractStorageView = HistoryView<ContractStorageAsHistory>;
pub type ContractStorageViewMut = HistoryViewMut<ContractStorageAsHistory>;

// #[async_trait]
// impl StorageViewRevetible for ContractStorageViewMut {
//     async fn revert_to(&self, block_number: u64) -> Result<(), DeoxysStorageError> {
//         todo!()
//         let db = Arc::new(DeoxysBackend::expose_db());

//         let mut read_options = ReadOptions::default();
//         read_options.set_iterate_lower_bound(bincode::serialize(&block_number)?);

//         // Currently we only use this iterator to retrieve the latest block number. A better way
//         to // do this would be to decouple the highest block lazy_static in `l2.rs`, but
//         in the // meantime this works as a workaround.
//         let mut iter = db.iterator_cf_opt(&db.get_column(Column::BlockStateDiff), read_options,
//         IteratorMode::End); let block_number_max = match iter.next() {
//             Some(Ok((bytes, _))) => bincode::deserialize(&bytes)
//                 .map_err(|_|
//         DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage))?,
//             Some(Err(_)) => return
//         Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage)),
//             None => 0,
//         };
//         assert!(block_number <= block_number_max);

//         // First, we aggregate all storage changes from the latest [StateDiff]
//         // up to the target block number. We use a non-blocking set and perform this
//         asychronously. // TODO: buffer this
//         let keys = (block_number..block_number_max).map(|key| bincode::serialize(&key).unwrap());
//         let change_set = Arc::new(SkipSet::new());
//         for entry in db.batched_multi_get_cf(&db.get_column(Column::BlockStateDiff), keys, true)
//         {     let state_diff = match entry {
//                 Ok(Some(bytes)) => bincode::deserialize::<StateDiff>(&bytes)
//                     .map_err(|_|
//         DeoxysStorageError::StorageDecodeError(StorageType::BlockStateDiff))?,
//                 _ => return
//         Err(DeoxysStorageError::StorageRetrievalError(StorageType::BlockStateDiff)),
//             };

//             let change_set = Arc::clone(&change_set);
//             spawn_blocking(move || {
//                 state_diff.storage_diffs.into_par_iter().for_each(
//                     |ContractStorageDiffItem { address, storage_entries }| {
//                         storage_entries.into_par_iter().for_each(|StorageEntry { key, value: _ }|
//         {                     change_set.insert((address, key));
//                         })
//                     },
//                 );
//             });
//         }

//         // Next, we iterate over all contract storage keys that have changed,
//         // reverting them to their state at the given block number
//         let mut set = JoinSet::new();
//         let change_set = Arc::try_unwrap(change_set).expect("Arc should not be aliased");
//         for key in change_set.into_iter() {
//             let db = Arc::clone(&db);
//             let key = (ContractAddress::from_field_element(key.0),
//         StorageKey::from_field_element(key.1));     let key_encoded = key.encode()?;

//             set.spawn(async move {
//                 let column = db.get_column(Column::ContractStorage);
//                 let Ok(result) = db.get_cf(&column, key_encoded.clone()) else {
//                     return
//         Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage));
//                 };
//                 let mut history: History<StarkFelt> = match result.map(|bytes|
//         Decode::decode(&bytes)) {             Some(Ok(history)) => history,
//                     Some(Err(_)) => return
//         Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractStorage)),
//                     None => unreachable!("Reverting contract storage should only use existing
//         contract addresses"),         };

//                 // history is updated and re-inserted into the db
//                 // or deleted if reverting leaves it empty
//                 history.revert_to(block_number);
//                 match history.is_empty() {
//                     true => {
//                         if db.delete(key_encoded.clone()).is_err() {
//                             return Err(DeoxysStorageError::StorageRevertError(
//                                 StorageType::ContractStorage,
//                                 block_number,
//                             ));
//                         }
//                     }
//                     false => {
//                         if db.put_cf(&column, key_encoded.clone(), history.encode()?).is_err() {
//                             return Err(DeoxysStorageError::StorageRevertError(
//                                 StorageType::ContractStorage,
//                                 block_number,
//                             ));
//                         }
//                     }
//                 }

//                 Ok(())
//             });
//         }

//         while let Some(res) = set.join_next().await {
//             res.unwrap()?;
//         }

//         Ok(())
//     }
// }
