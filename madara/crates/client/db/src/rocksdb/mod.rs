use crate::{
    db::{ClassInfoWithBlockN, CompiledSierraWithBlockN, DBBackend, TxIndex},
    MadaraStorageError,
};
use mp_block::{MadaraBlockInfo, TransactionWithReceipt};
use mp_convert::Felt;
use mp_state_update::StateDiff;
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, WriteOptions};
use std::{ops::RangeBounds, path::Path, sync::Arc};

mod blocks;
mod classes;
mod column;
pub(crate) mod iter_pinned;
mod options;
mod state;

pub type DB = DBWithThreadMode<MultiThreaded>;

pub use column::*;
pub use options::*;

pub struct RocksDBBackend {
    pub(crate) db: DB,
    pub(crate) write_opt_no_wal: WriteOptions,
}

impl RocksDBBackend {
    pub fn new(db: DB) -> Self {
        let mut write_opt_no_wal = WriteOptions::new();
        write_opt_no_wal.disable_wal(true);
        Self { db, write_opt_no_wal }
    }

    pub fn open(path: &Path, config: &RocksDBConfig) -> anyhow::Result<Self> {
        let opts = rocksdb_global_options(config)?;
        tracing::debug!("opening db at {:?}", path.display());
        let db = DB::open_cf_descriptors(
            &opts,
            path,
            ALL_COLUMNS.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(config))),
        )?;
        Ok(Self::new(db))
    }

    pub(crate) fn get_column(&self, col: &Column) -> Arc<BoundColumnFamily<'_>> {
        let name = col.rocksdb_name;
        match self.db.cf_handle(name) {
            Some(column) => column,
            None => panic!("column {name} not initialized"),
        }
    }
}

impl DBBackend for RocksDBBackend {
    fn get_storage_at(
        &self,
        block_n: u64,
        contract_address: &Felt,
        key: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        self.get_storage_at_impl(block_n, contract_address, key)
    }
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>, MadaraStorageError> {
        self.get_contract_nonce_at_impl(block_n, contract_address)
    }
    fn get_contract_class_hash_at(
        &self,
        block_n: u64,
        contract_address: &Felt,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        self.get_contract_class_hash_at_impl(block_n, contract_address)
    }
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool, MadaraStorageError> {
        self.is_contract_deployed_at_impl(block_n, contract_address)
    }

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>, MadaraStorageError> {
        self.find_block_hash_impl(block_hash)
    }
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<TxIndex>, MadaraStorageError> {
        self.find_transaction_hash_impl(tx_hash)
    }
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
        self.get_block_info_impl(block_n)
    }
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>, MadaraStorageError> {
        self.get_block_state_diff_impl(block_n)
    }
    fn get_transaction(
        &self,
        block_n: u64,
        tx_index: u64,
    ) -> Result<Option<TransactionWithReceipt>, MadaraStorageError> {
        self.get_transaction_impl(block_n, tx_index)
    }
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt, MadaraStorageError>> + '_ {
        self.get_block_transactions_impl(block_n, from_tx_index)
    }

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>, MadaraStorageError> {
        self.get_class_impl(class_hash)
    }
    fn get_class_compiled(
        &self,
        compiled_class_hash: &Felt,
    ) -> Result<Option<CompiledSierraWithBlockN>, MadaraStorageError> {
        self.get_class_compiled_impl(compiled_class_hash)
    }
    fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        self.get_filtered_events_impl(filter)
    }
}
