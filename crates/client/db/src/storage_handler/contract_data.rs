use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use crossbeam_skiplist::SkipMap;
use mp_contract::class::StorageContractData;
use parity_scale_codec::{Decode, Encode};
use rocksdb::{IteratorMode, WriteBatchWithTransaction};
use starknet_api::core::{ClassHash, ContractAddress};
use tokio::task::JoinSet;

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractDataViewMut(SkipMap<ContractAddress, StorageContractData>);
pub struct ContractDataView;

impl StorageView for ContractDataView {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.last_key_value().map(|(_k, v)| v).cloned())
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db.key_may_exist_cf(&column, contract_address.encode()) {
            true => Ok(self.get(contract_address)?.is_some()),
            false => Ok(false),
        }
    }
}

impl ContractDataView {
    pub fn get_at(
        self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<Option<StorageContractData>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.range(..=block_number).next_back().map(|(_, value)| value).cloned())
    }
}

impl StorageView for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        if let Some(entry) = self.0.get(contract_address) {
            return Ok(Some(entry.value().clone()));
        }

        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let tree: BTreeMap<u64, StorageContractData> = match db
            .get_cf(&column, contract_address.encode())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => BTreeMap::new(),
        };

        Ok(tree.last_key_value().map(|(_k, v)| v).cloned())
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db.key_may_exist_cf(&column, contract_address.encode()) {
            true => Ok(self.get(contract_address)?.is_some()),
            false => Ok(false),
        }
    }
}

#[async_trait]
impl StorageViewMut for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn insert(&self, contract_address: Self::KEY, contract_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(contract_address, contract_data);
        Ok(())
    }

    async fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        // let column = db.get_column(Column::ContractData);
        // let (keys, value): (Vec<_>, Vec<_>) = self.0.into_iter().unzip();
        // let keys_cf = keys.iter().map(|key| (&column, bincode::serialize(key).unwrap()));
        // let histories = db.multi_get_cf(keys_cf).into_iter().map(|result| {
        //     result.and_then(|option| match option {
        //         Some(bytes) => bincode::deserialize(&bytes)
        //             .map_err(|_|
        // DeoxysStorageError::StorageDecodeError(StorageType::ContractData)),         None
        // => todo!(),     })
        // });

        let mut set = JoinSet::new();
        for (key, value) in self.0.into_iter() {
            let db = Arc::clone(db);

            set.spawn(async move {
                let column = db.get_column(Column::ContractData);

                let Ok(tree) = db.get_cf(&column, key.encode()) else {
                    return Err(DeoxysStorageError::StorageRetrievalError(StorageType::ContractData));
                };

                let mut tree: BTreeMap<u64, StorageContractData> = match tree {
                    Some(bytes) => match BTreeMap::decode(&mut &bytes[..]) {
                        Ok(tree) => tree,
                        Err(_) => return Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractData)),
                    },
                    None => BTreeMap::new(),
                };

                tree.insert(block_number, value);
                if db.put_cf(&column, key.encode(), tree.encode()).is_err() {
                    return Err(DeoxysStorageError::StorageCommitError(StorageType::ContractData));
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

#[async_trait()]
impl StorageViewRevetible for ContractDataViewMut {
    async fn revert_to(&self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let iterator = db.iterator_cf(&column, IteratorMode::Start);

        for data in iterator {
            let (key, value) =
                data.map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?;
            let mut tree: BTreeMap<u64, ClassHash> = BTreeMap::decode(&mut &value[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?;

            tree.retain(|&k, _| k <= block_number);

            db.put_cf(&column, key, tree.encode())
                .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractData, block_number))?;
        }

        Ok(())
    }
}

impl ContractDataViewMut {
    pub fn commit_sync(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        for (key, value) in self.0.into_iter() {
            let mut tree: BTreeMap<u64, StorageContractData> = match db
                .get_cf(&column, key.encode())
                .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
            {
                Some(bytes) => BTreeMap::decode(&mut &bytes[..])
                    .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
                None => BTreeMap::new(),
            };

            tree.insert(block_number, value);
            db.put_cf(&column, key.encode(), tree.encode())
                .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractData))?;
        }

        Ok(())
    }
}
