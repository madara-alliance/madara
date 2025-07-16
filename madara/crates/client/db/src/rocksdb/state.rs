use crate::{
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBBackend},
    MadaraStorageError,
};
use mp_convert::Felt;
use rocksdb::{IteratorMode, ReadOptions};

// NB: Columns cf needs prefix extractor of these length during creation
const CONTRACT_STORAGE_PREFIX_LEN: usize = 64;
const CONTRACT_NONCES_PREFIX_LEN: usize = 32;
const CONTRACT_CLASS_HASH_PREFIX_LEN: usize = 32;

pub(crate) const CONTRACT_STORAGE_COLUMN: &Column =
    &Column::new("contract_storage").with_prefix_extractor_len(CONTRACT_STORAGE_PREFIX_LEN);
pub(crate) const CONTRACT_NONCE_COLUMN: &Column =
    &Column::new("contract_nonces").with_prefix_extractor_len(CONTRACT_NONCES_PREFIX_LEN);
pub(crate) const CONTRACT_CLASS_HASH_COLUMN: &Column =
    &Column::new("contract_class_hashes").with_prefix_extractor_len(CONTRACT_CLASS_HASH_PREFIX_LEN);

// prefix [<contract_address 32 bytes> | <storage_key 32 bytes>] | <block_n 4 bytes>
const STORAGE_KEY_LEN: usize = 32 * 2 + size_of::<u32>();
fn make_storage_column_key(
    contract_address: &Felt,
    storage_key: &Felt,
    block_n: u64,
) -> Result<[u8; STORAGE_KEY_LEN], MadaraStorageError> {
    let block_n = u32::MAX - u32::try_from(block_n).map_err(|_| MadaraStorageError::InvalidBlockNumber)?;
    let mut key = [0u8; STORAGE_KEY_LEN];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..2 * 32].copy_from_slice(&storage_key.to_bytes_be());
    key[2 * 32..].copy_from_slice(&block_n.to_be_bytes());
    Ok(key)
}

// prefix [<contract_address 32 bytes>] | <block_n 4 bytes>
const CONTRACT_KEY_LEN: usize = 32 * 2 + size_of::<u32>();
fn make_contract_column_key(
    contract_address: &Felt,
    block_n: u64,
) -> Result<[u8; CONTRACT_KEY_LEN], MadaraStorageError> {
    let block_n = u32::MAX - u32::try_from(block_n).map_err(|_| MadaraStorageError::InvalidBlockNumber)?;
    let mut key = [0u8; CONTRACT_KEY_LEN];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..].copy_from_slice(&block_n.to_be_bytes());
    Ok(key)
}

impl RocksDBBackend {
    fn db_history_kv_resolve<V: serde::de::DeserializeOwned + 'static>(
        &self,
        bin_prefix: &[u8],
        col: &Column,
    ) -> Result<Option<V>, MadaraStorageError> {
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let mode = IteratorMode::From(&bin_prefix, rocksdb::Direction::Forward);
        let mut iter = DBIterator::new_cf(&self.db, &self.get_column(col), options, mode)
            .into_iter_values(|bytes| bincode::deserialize(bytes));
        let n = iter.next();

        Ok(n.transpose()?.transpose()?)
    }

    fn db_history_kv_contains(&self, bin_prefix: &[u8], col: &Column) -> Result<bool, MadaraStorageError> {
        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let mode = IteratorMode::From(&bin_prefix, rocksdb::Direction::Forward);
        let mut iter = DBIterator::new_cf(&self.db, &self.get_column(col), options, mode);
        Ok(iter.next()?)
    }

    #[tracing::instrument(skip(self), fields(module = "ContractDB"))]
    pub(super) fn get_storage_at_impl(
        &self,
        block_n: u64,
        contract_address: &Felt,
        key: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        let prefix = make_storage_column_key(contract_address, key, block_n)?;
        self.db_history_kv_resolve(&prefix, CONTRACT_STORAGE_COLUMN)
    }

    #[tracing::instrument(skip(self), fields(module = "ContractDB"))]
    pub(super) fn get_contract_nonce_at_impl(
        &self,
        block_n: u64,
        contract_address: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        let prefix = make_contract_column_key(contract_address, block_n)?;
        self.db_history_kv_resolve(&prefix, CONTRACT_NONCE_COLUMN)
    }

    #[tracing::instrument(skip(self), fields(module = "ContractDB"))]
    pub(super) fn get_contract_class_hash_at_impl(
        &self,
        block_n: u64,
        contract_address: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        let prefix = make_contract_column_key(contract_address, block_n)?;
        self.db_history_kv_resolve(&prefix, CONTRACT_CLASS_HASH_COLUMN)
    }

    #[tracing::instrument(skip(self), fields(module = "ContractDB"))]
    pub(super) fn is_contract_deployed_at_impl(
        &self,
        block_n: u64,
        contract_address: &Felt,
    ) -> Result<bool, MadaraStorageError> {
        let prefix = make_contract_column_key(contract_address, block_n)?;
        self.db_history_kv_contains(&prefix, CONTRACT_CLASS_HASH_COLUMN)
    }
}
