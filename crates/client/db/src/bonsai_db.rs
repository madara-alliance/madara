use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, BonsaiStorageConfig, DatabaseKey};
use rocksdb::{
    Direction, IteratorMode, OptimisticTransactionOptions, ReadOptions, SnapshotWithThreadMode, Transaction,
    WriteBatchWithTransaction, WriteOptions,
};
use starknet_types_core::hash::StarkHash;

use crate::{BonsaiDbError, Column, DatabaseExt, DB};

pub type RocksDBTransaction = WriteBatchWithTransaction<true>;

#[derive(Clone, Debug)]
pub(crate) struct DatabaseKeyMapping {
    pub(crate) flat: Column,
    pub(crate) trie: Column,
    pub(crate) trie_log: Column,
}

impl DatabaseKeyMapping {
    pub(crate) fn map(&self, key: &DatabaseKey) -> Column {
        match key {
            DatabaseKey::Trie(_) => self.trie,
            DatabaseKey::Flat(_) => self.flat,
            DatabaseKey::TrieLog(_) => self.trie_log,
        }
    }
}

/// This structs [`Deref`]s to a readonly [`BonsaiStorage`] for read access,
/// and creates new [`BonsaiStorage`] for write access.
///
/// If you want the commits to [`BonsaiStorage`] to be written into a local transaction without
/// sending it to rocksdb, use [`BonsaiStorageAccess::transactional`]. You can then commit the
/// changes to rocksdb with [`TransactionDb::commit_transaction`].
pub struct BonsaiStorageAccess<H: StarkHash + Send + Sync> {
    db: Arc<DB>,
    column_mapping: DatabaseKeyMapping,
    readonly: BonsaiStorage<BasicId, BonsaiDbForRead, H>,
}
impl<H: StarkHash + Send + Sync> BonsaiStorageAccess<H> {
    pub(crate) fn new(db: Arc<DB>, column_mapping: DatabaseKeyMapping) -> Self {
        let bonsai_db = BonsaiDbForRead::new(db.clone(), column_mapping.clone());
        Self { db, column_mapping, readonly: Self::make_clone(bonsai_db) }
    }

    fn make_clone<D: BonsaiDatabase>(bonsai_db: D) -> BonsaiStorage<BasicId, D, H> {
        BonsaiStorage::new(bonsai_db, BonsaiStorageConfig { ..Default::default() })
            .expect("failed to create bonsai storage")
    }

    /// Access the bonsai storage in for writing.
    pub fn writable(&self) -> BonsaiStorage<BasicId, BonsaiDb<'_>, H> {
        Self::make_clone(BonsaiDb::new(&self.db, self.column_mapping.clone()))
    }
}

impl<H: StarkHash + Send + Sync> Deref for BonsaiStorageAccess<H> {
    type Target = BonsaiStorage<BasicId, BonsaiDbForRead, H>;

    /// Access the bonsai storage in for reading.
    fn deref(&self) -> &Self::Target {
        &self.readonly
    }
}

pub struct BonsaiDbForRead {
    /// Database interface for key-value operations.
    db: Arc<DB>,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
}

impl BonsaiDbForRead {
    pub(crate) fn new(db: Arc<DB>, column_mapping: DatabaseKeyMapping) -> Self {
        Self { db, column_mapping }
    }
}

impl BonsaiDatabase for BonsaiDbForRead {
    type Batch = RocksDBTransaction;
    type DatabaseError = BonsaiDbError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice())?)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        Ok(iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) { Some((key.to_vec(), value.to_vec())) } else { None }
                } else {
                    None
                }
            })
            .collect())
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        log::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    fn insert(
        &mut self,
        _key: &DatabaseKey,
        _value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        unimplemented!()
    }
    fn remove(
        &mut self,
        _key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        unimplemented!()
    }
    fn remove_by_prefix(&mut self, _prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        unimplemented!()
    }
    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        unimplemented!()
    }
}

/// [`Clone`] does not clone snapshot state.
pub struct BonsaiDb<'db> {
    /// Database interface for key-value operations.
    db: &'db DB,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
    snapshots: BTreeMap<BasicId, SnapshotWithThreadMode<'db, DB>>,
}

impl<'db> Clone for BonsaiDb<'db> {
    fn clone(&self) -> Self {
        Self { db: self.db, column_mapping: self.column_mapping.clone(), snapshots: Default::default() }
    }
}

impl<'db> fmt::Debug for BonsaiDb<'db> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BonsaiDb").field("db", &self.db).field("column_mapping", &self.column_mapping).finish()
    }
}

impl<'db> BonsaiDb<'db> {
    pub(crate) fn new(db: &'db DB, column_mapping: DatabaseKeyMapping) -> Self {
        Self { db, column_mapping, snapshots: Default::default() }
    }
}

impl<'db> BonsaiDatabase for BonsaiDb<'db> {
    type Batch = RocksDBTransaction;
    type DatabaseError = BonsaiDbError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice())?)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        Ok(iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) { Some((key.to_vec(), value.to_vec())) } else { None }
                } else {
                    None
                }
            })
            .collect())
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        log::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.db.get_column(self.column_mapping.map(key));
        let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.db.put_cf(&handle, key.as_slice(), value)?;
        }
        Ok(old_value)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.db.delete_cf(&handle, key.as_slice())?;
        }
        Ok(old_value)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        let mut batch = self.create_batch();
        for kv in iter {
            if let Ok((key, _)) = kv {
                if key.starts_with(prefix.as_slice()) {
                    batch.delete_cf(&handle, &key);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        drop(handle);
        self.write_batch(batch)?;
        Ok(())
    }

    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(self.db.write(batch)?)
    }
}

pub struct BonsaiTransaction<'db> {
    txn: Transaction<'db, DB>,
    db: &'db DB,
    column_mapping: DatabaseKeyMapping,
}

impl<'db> BonsaiDatabase for BonsaiTransaction<'db> {
    type Batch = WriteBatchWithTransaction<true>;
    type DatabaseError = BonsaiDbError;

    fn create_batch(&self) -> Self::Batch {
        self.txn.get_writebatch()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.txn.get_cf(&handle, key.as_slice())?)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.txn.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        Ok(iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) { Some((key.to_vec(), value.to_vec())) } else { None }
                } else {
                    None
                }
            })
            .collect())
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        log::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.txn.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.db.get_column(self.column_mapping.map(key));
        let old_value = self.txn.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.txn.put_cf(&handle, key.as_slice(), value)?;
        }
        Ok(old_value)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        log::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        let old_value = self.txn.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.txn.delete_cf(&handle, key.as_slice())?;
        }
        Ok(old_value)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.txn.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        let mut batch = self.create_batch();
        for kv in iter {
            if let Ok((key, _)) = kv {
                if key.starts_with(prefix.as_slice()) {
                    batch.delete_cf(&handle, &key);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        drop(handle);
        self.write_batch(batch)?;
        Ok(())
    }

    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(self.txn.rebuild_from_writebatch(&batch)?)
    }
}

impl<'db> BonsaiPersistentDatabase<BasicId> for BonsaiDb<'db>
where
    Self: 'db,
{
    type Transaction = BonsaiTransaction<'db>;
    type DatabaseError = BonsaiDbError;

    fn snapshot(&mut self, id: BasicId) {
        log::trace!("Generating RocksDB snapshot");
        let snapshot = self.db.snapshot();
        self.snapshots.insert(id, snapshot);
        // TODO: snapshot limits
        // if let Some(max_number_snapshot) = self.config.max_saved_snapshots {
        //     while self.snapshots.len() > max_number_snapshot {
        //         self.snapshots.pop_first();
        //     }
        // }
    }

    fn transaction(&self, id: BasicId) -> Option<Self::Transaction> {
        log::trace!("Generating RocksDB transaction");
        if let Some(snapshot) = self.snapshots.get(&id) {
            let write_opts = WriteOptions::default();
            let mut txn_opts = OptimisticTransactionOptions::default();
            txn_opts.set_snapshot(true);
            let txn = self.db.transaction_opt(&write_opts, &txn_opts);

            let mut read_options = ReadOptions::default();
            read_options.set_snapshot(snapshot);

            Some(BonsaiTransaction { txn, db: self.db, column_mapping: self.column_mapping.clone() })
        } else {
            None
        }
    }

    fn merge(&mut self, transaction: Self::Transaction) -> Result<(), Self::DatabaseError> {
        transaction.txn.commit()?;
        Ok(())
    }
}
