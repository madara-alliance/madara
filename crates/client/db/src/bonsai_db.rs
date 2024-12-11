use crate::error::DbError;
use crate::snapshots::{SnapshotRef, Snapshots};
use crate::{Column, DatabaseExt, WriteBatchWithTransaction, DB};
use bonsai_trie::id::{BasicId, Id};
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, ByteVec, DatabaseKey};
use rocksdb::{Direction, IteratorMode, WriteOptions};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub type GlobalTrie<H> = BonsaiStorage<BasicId, BonsaiDb, H>;

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

pub struct BonsaiDb {
    db: Arc<DB>,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
    snapshots: Arc<Snapshots>,
    write_opt: WriteOptions,
}

impl BonsaiDb {
    pub(crate) fn new(db: Arc<DB>, snapshots: Arc<Snapshots>, column_mapping: DatabaseKeyMapping) -> Self {
        let mut write_opt = WriteOptions::default();
        write_opt.disable_wal(true);
        Self { db, column_mapping, write_opt, snapshots }
    }
}

impl fmt::Debug for BonsaiDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BonsaiDb {{}}")
    }
}

impl BonsaiDatabase for BonsaiDb {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = DbError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    #[tracing::instrument(skip(self, key), fields(module = "BonsaiDB"))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix), fields(module = "BonsaiDB"))]
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        tracing::trace!("Getting by prefix from RocksDB: {:?}", prefix);
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

    #[tracing::instrument(skip(self, key), fields(module = "BonsaiDB"))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        Ok(self.db.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    #[tracing::instrument(skip(self, key, value, batch), fields(module = "BonsaiDB"))]
    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.db.get_column(self.column_mapping.map(key));

        let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.db.put_cf_opt(&handle, key.as_slice(), value, &self.write_opt)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, key, batch), fields(module = "BonsaiDB"))]
    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.db.get_column(self.column_mapping.map(key));
        let old_value = self.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.db.delete_cf_opt(&handle, key.as_slice(), &self.write_opt)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix), fields(module = "BonsaiDB"))]
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", prefix);
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

    #[tracing::instrument(skip(self, batch), fields(module = "BonsaiDB"))]
    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(self.db.write_opt(batch, &self.write_opt)?)
    }
}

fn to_changed_key(k: &DatabaseKey) -> (u8, ByteVec) {
    (
        match k {
            DatabaseKey::Trie(_) => 0,
            DatabaseKey::Flat(_) => 1,
            DatabaseKey::TrieLog(_) => 2,
        },
        k.as_slice().into(),
    )
}

/// The backing database for a bonsai storage view. This is used
/// to implement historical access (for storage proofs), by applying
/// changes from the trie-log without modifying the real database.
/// This is kind of a hack for now. This abstraction shouldn't look like
/// this at all ideally, and it should probably be an implementation
/// detail of bonsai-trie.
pub struct BonsaiTransaction {
    /// Backing snapshot. If the value has not been changed, it'll be queried from
    /// here.
    snapshot: SnapshotRef,
    /// The changes on top of the snapshot.
    /// Key is (column id, key) and value is Some(value) if the change is an insert, and None
    /// if the change is a deletion of the key.
    changed: BTreeMap<(u8, ByteVec), Option<ByteVec>>,
    column_mapping: DatabaseKeyMapping,
}

impl fmt::Debug for BonsaiTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BonsaiTransaction {{}}")
    }
}

// TODO: a lot of this is not really used yet, this whole abstraction does not really make sense anyway, this needs to be modified
// upstream in bonsai-trie
impl BonsaiDatabase for BonsaiTransaction {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = DbError;

    fn create_batch(&self) -> Self::Batch {
        Default::default()
    }

    #[tracing::instrument(skip(self, key), fields(module = "BonsaiDB"))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        if let Some(val) = self.changed.get(&to_changed_key(key)) {
            return Ok(val.clone());
        }
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key));
        Ok(self.snapshot.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        unreachable!("unused for now")
    }

    #[tracing::instrument(skip(self, key), fields(module = "BonsaiDB"))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key));
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.is_some())
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(None)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        self.changed.insert(to_changed_key(key), None);
        Ok(None)
    }

    fn remove_by_prefix(&mut self, _prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        unreachable!("unused yet")
    }

    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(())
    }
}

impl BonsaiPersistentDatabase<BasicId> for BonsaiDb {
    type Transaction<'a>
        = BonsaiTransaction
    where
        Self: 'a;
    type DatabaseError = DbError;

    /// this is called upstream, but we ignore it for now because we create the snapshot in [`crate::MadaraBackend::store_block`]
    #[tracing::instrument(skip(self), fields(module = "BonsaiDB"))]
    fn snapshot(&mut self, id: BasicId) {}

    #[tracing::instrument(skip(self), fields(module = "BonsaiDB"))]
    fn transaction(&self, requested_id: BasicId) -> Option<(BasicId, Self::Transaction<'_>)> {
        tracing::trace!("Generating RocksDB transaction");
        let (id, snapshot) = self.snapshots.get_closest(requested_id.as_u64());

        tracing::debug!("Snapshot for requested block_id={requested_id:?} => got block_id={id:?}");

        id.map(|id| {
            (
                BasicId::new(id),
                BonsaiTransaction {
                    snapshot,
                    column_mapping: self.column_mapping.clone(),
                    changed: Default::default(),
                },
            )
        })
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("unused for now")
    }
}
