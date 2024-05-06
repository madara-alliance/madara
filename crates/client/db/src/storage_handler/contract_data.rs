use async_trait::async_trait;
use crossbeam_skiplist::SkipMap;
use itertools::izip;
use rocksdb::{IteratorMode, WriteBatchWithTransaction};
use starknet_api::core::{ClassHash, ContractAddress, Nonce};

use super::primitives::contract::StorageContractData;
use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::{Column, DatabaseExt, DeoxysBackend};

#[derive(Default, Debug)]
pub struct ContractDataViewMut(SkipMap<ContractAddress, (Option<ClassHash>, Option<Nonce>)>);
pub struct ContractDataView;

impl StorageView for ContractDataView {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db
            .get_cf(&column, bincode::serialize(&contract_address)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => Ok(Some(
                bincode::deserialize::<StorageContractData>(&bytes[..])
                    .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            )),
            None => Ok(None),
        }
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db.key_may_exist_cf(&column, bincode::serialize(&contract_address)?) {
            true => Ok(self.get(contract_address)?.is_some()),
            false => Ok(false),
        }
    }
}

impl ContractDataView {
    pub fn get_nonce(&self, contract_address: &ContractAddress) -> Result<Option<Nonce>, DeoxysStorageError> {
        self.get(contract_address)
            .map(|option| option.map(|contract_data| contract_data.nonce.get().cloned().unwrap_or_default()))
    }

    pub fn get_class_hash(&self, contract_address: &ContractAddress) -> Result<Option<ClassHash>, DeoxysStorageError> {
        self.get(contract_address)
            .map(|option| option.and_then(|contract_data| contract_data.class_hash.get().cloned()))
    }

    pub fn get_nonce_at(
        &self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<Option<Nonce>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let contract_data = match db
            .get_cf(&column, bincode::serialize(&contract_address)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => bincode::deserialize::<StorageContractData>(&bytes[..])?,
            None => StorageContractData::default(),
        };

        Ok(contract_data.nonce.get_at(block_number).cloned())
    }

    pub fn get_class_hash_at(
        &self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<Option<ClassHash>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        let contract_data = match db
            .get_cf(&column, bincode::serialize(&contract_address)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => bincode::deserialize::<StorageContractData>(&bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            None => StorageContractData::default(),
        };

        Ok(contract_data.class_hash.get_at(block_number).cloned())
    }

    pub fn is_contract_deployed_at(
        &self,
        contract_address: &ContractAddress,
        block_number: u64,
    ) -> Result<bool, DeoxysStorageError> {
        match self.get(contract_address)? {
            Some(contract_data) => Ok(contract_data.class_hash.get_at(block_number).is_some()),
            None => Ok(false),
        }
    }
}

impl StorageViewMut for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = (Option<ClassHash>, Option<Nonce>);

    fn insert(&self, contract_address: Self::KEY, contract_data: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(contract_address, contract_data);
        Ok(())
    }

    fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);
        let (keys, values): (Vec<_>, Vec<_>) = self.0.into_iter().unzip();
        let keys_cf = keys.iter().map(|key| (&column, bincode::serialize(key).unwrap()));
        let histories = db
            .multi_get_cf(keys_cf)
            .into_iter()
            .map(|result| {
                result.map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData)).and_then(
                    |option| match option {
                        Some(bytes) => bincode::deserialize::<StorageContractData>(&bytes)
                            .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData)),
                        None => Ok(StorageContractData::default()),
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut batch = WriteBatchWithTransaction::<true>::default();
        for (key, mut contract_data, (class_hash, nonce)) in izip!(keys, histories, values) {
            if let Some(class_hash) = class_hash {
                contract_data.class_hash.push(block_number, class_hash)?;
            }

            if let Some(nonce) = nonce {
                contract_data.nonce.push(block_number, nonce)?;
            }

            batch.put_cf(&column, bincode::serialize(&key)?, bincode::serialize(&contract_data)?);
        }
        db.write(batch).map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractData))
    }
}

impl StorageView for ContractDataViewMut {
    type KEY = ContractAddress;
    type VALUE = StorageContractData;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db
            .get_cf(&column, bincode::serialize(&contract_address)?)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractData))?
        {
            Some(bytes) => Ok(Some(
                bincode::deserialize::<StorageContractData>(&bytes[..])
                    .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?,
            )),
            None => Ok(None),
        }
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        let db = DeoxysBackend::expose_db();
        let column = db.get_column(Column::ContractData);

        match db.key_may_exist_cf(&column, bincode::serialize(&contract_address)?) {
            true => Ok(self.get(contract_address)?.is_some()),
            false => Ok(false),
        }
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
            let mut contract_data = bincode::deserialize::<StorageContractData>(&value[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractData))?;

            contract_data.class_hash.revert_to(block_number);
            contract_data.nonce.revert_to(block_number);

            match (contract_data.class_hash.is_empty(), contract_data.nonce.is_empty()) {
                (true, true) => db
                    .delete(key)
                    .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractData, block_number))?,
                _ => db
                    .put_cf(&column, key, bincode::serialize(&contract_data)?)
                    .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractData, block_number))?,
            }
        }

        Ok(())
    }
}

impl ContractDataViewMut {
    pub fn insert_nonce(&self, contract_address: ContractAddress, nonce: Nonce) -> Result<(), DeoxysStorageError> {
        self.insert(contract_address, (None, Some(nonce)))
    }

    pub fn insert_class_hash(
        &self,
        contract_address: ContractAddress,
        class_hash: ClassHash,
    ) -> Result<(), DeoxysStorageError> {
        self.insert(contract_address, (Some(class_hash), None))
    }
}
