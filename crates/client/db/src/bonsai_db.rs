use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, ByteVec, DatabaseKey};
use rocksdb::{Direction, IteratorMode, WriteOptions};

use crate::error::DbError;
use crate::{Column, DatabaseExt, WriteBatchWithTransaction, DB};

#[derive(Clone, Debug)]
pub(crate) struct DatabaseKeyMapping {
    pub(crate) flat: Column,
    pub(crate) trie: Column,
    pub(crate) log: Column,
}

impl DatabaseKeyMapping {
    pub(crate) fn map(&self, key: &DatabaseKey) -> Column {
        match key {
            DatabaseKey::Trie(_) => self.trie,
            DatabaseKey::Flat(_) => self.flat,
            DatabaseKey::TrieLog(_) => self.log,
        }
    }
}

pub struct BonsaiDb<'db> {
    db: &'db DB,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
    // snapshots: BTreeMap<BasicId, SnapshotWithThreadMode<'db, DB>>,
    write_opt: WriteOptions,
}

impl<'db> BonsaiDb<'db> {
    pub(crate) fn new(db: &'db DB, column_mapping: DatabaseKeyMapping) -> Self {
        let mut write_opt = WriteOptions::default();
        write_opt.disable_wal(true);
        Self { db, column_mapping, write_opt }
    }
}

impl BonsaiDatabase for BonsaiDb<'_> {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = DbError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        log::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.db.get_column(self.column_mapping.map(prefix));
        let iter = self.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        Ok(iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) {
                        // nb: to_vec on a Box<[u8]> is a noop conversion
                        Some((key.to_vec().into(), value.to_vec().into()))
                    } else {
                        None
                    }
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
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        log::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.db.get_column(self.column_mapping.map(key));

        // NB: we don't need old value as the trie log is not used :)
        // this actually speeds up things quite a lot

        // let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.db.put_cf_opt(&handle, key.as_slice(), value, &self.write_opt)?;
        }
        Ok(None)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        log::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        // let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.db.delete_cf_opt(&handle, key.as_slice(), &self.write_opt)?;
        }
        Ok(None)
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
        Ok(self.db.write_opt(batch, &self.write_opt)?)
    }
}

// pub struct BonsaiTransaction<'db> {
//     txn: Transaction<'db, DB>,
//     db: &'db DB,
//     column_mapping: DatabaseKeyMapping,
// }

// impl<'db> BonsaiDatabase for BonsaiTransaction<'db> {
//     type Batch = WriteBatchWithTransaction<true>;
//     type DatabaseError = MadaraStorageError;

//     fn create_batch(&self) -> Self::Batch {
//         self.txn.get_writebatch()
//     }

//     fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
//         log::trace!("Getting from RocksDB: {:?}", key);
//         let handle = self.db.get_column(self.column_mapping.map(key));
//         Ok(self.txn.get_cf(&handle, key.as_slice())?)
//     }

//     fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
//         log::trace!("Getting from RocksDB: {:?}", prefix);
//         let handle = self.db.get_column(self.column_mapping.map(prefix));
//         let iter = self.txn.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
//         Ok(iter
//             .map_while(|kv| {
//                 if let Ok((key, value)) = kv {
//                     if key.starts_with(prefix.as_slice()) {
//                         Some((key.to_vec(), value.to_vec()))
//                     } else {
//                         None
//                     }
//                 } else {
//                     None
//                 }
//             })
//             .collect())
//     }

//     fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
//         log::trace!("Checking if RocksDB contains: {:?}", key);
//         let handle = self.db.get_column(self.column_mapping.map(key));
//         Ok(self.txn.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
//     }

//     fn insert(
//         &mut self,
//         key: &DatabaseKey,
//         value: &[u8],
//         batch: Option<&mut Self::Batch>,
//     ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
//         log::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
//         let handle = self.db.get_column(self.column_mapping.map(key));
//         let old_value = self.txn.get_cf(&handle, key.as_slice())?;
//         if let Some(batch) = batch {
//             batch.put_cf(&handle, key.as_slice(), value);
//         } else {
//             self.txn.put_cf(&handle, key.as_slice(), value)?;
//         }
//         Ok(old_value)
//     }

//     fn remove(
//         &mut self,
//         key: &DatabaseKey,
//         batch: Option<&mut Self::Batch>,
//     ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
//         log::trace!("Removing from RocksDB: {:?}", key);
//         let handle = self.db.get_column(self.column_mapping.map(key));
//         let old_value = self.txn.get_cf(&handle, key.as_slice())?;
//         if let Some(batch) = batch {
//             batch.delete_cf(&handle, key.as_slice());
//         } else {
//             self.txn.delete_cf(&handle, key.as_slice())?;
//         }
//         Ok(old_value)
//     }

//     fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
//         log::trace!("Getting from RocksDB: {:?}", prefix);
//         let handle = self.db.get_column(self.column_mapping.map(prefix));
//         let iter = self.txn.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
//         let mut batch = self.create_batch();
//         for kv in iter {
//             if let Ok((key, _)) = kv {
//                 if key.starts_with(prefix.as_slice()) {
//                     batch.delete_cf(&handle, &key);
//                 } else {
//                     break;
//                 }
//             } else {
//                 break;
//             }
//         }
//         drop(handle);
//         self.write_batch(batch)?;
//         Ok(())
//     }

//     fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
//         Ok(self.txn.rebuild_from_writebatch(&batch)?)
//     }
// }

impl<'db> BonsaiPersistentDatabase<BasicId> for BonsaiDb<'db>
where
    Self: 'db,
{
    type Transaction = Self;
    type DatabaseError = DbError;

    fn snapshot(&mut self, _id: BasicId) {
        log::trace!("Generating RocksDB snapshot");
        // let snapshot = self.db.snapshot();
        // self.snapshots.insert(id, snapshot);
    }

    fn transaction(&self, _id: BasicId) -> Option<Self::Transaction> {
        log::trace!("Generating RocksDB transaction");
        // TODO: we lie about supporting transactions here
        Some(BonsaiDb::new(self.db, self.column_mapping.clone()))
        // if let Some(snapshot) = self.snapshots.get(&id) {
        //     let write_opts = WriteOptions::default();
        //     let mut txn_opts = OptimisticTransactionOptions::default();
        //     txn_opts.set_snapshot(true);
        //     let txn = self.db.transaction_opt(&write_opts, &txn_opts);

        //     let mut read_options = ReadOptions::default();
        //     read_options.set_snapshot(snapshot);

        //     Some(BonsaiTransaction { txn, db: self.db, column_mapping: self.column_mapping.clone() })
        // } else {
        //     None
        // }
    }

    fn merge(&mut self, _transaction: Self::Transaction) -> Result<(), Self::DatabaseError> {
        // transaction.txn.commit()?;
        Ok(())
    }
}
