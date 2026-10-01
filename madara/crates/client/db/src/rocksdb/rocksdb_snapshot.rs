use librocksdb_sys as ffi;
use rocksdb::{AsColumnFamilyRef, DBAccess, DBPinnableSlice, Error, IteratorMode, ReadOptions};
use std::sync::Arc;

use crate::rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner};

/// A copy of [`rocksdb::SnapshotWithThreadMode`] with an Arc<DB> instead of an &'_ DB reference
/// The reason this has to exist is because we want to store Snapshots inside the MadaraBackend. For that to work, we need
/// to get rid of the lifetime in Snapshot, and replace it with an Arc.
///
/// See [rust-rocksdb/rust-rocksdb#937](https://github.com/rust-rocksdb/rust-rocksdb/issues/937) and
/// [rust-rocksdb/rust-rocksdb#936](https://github.com/rust-rocksdb/rust-rocksdb/issues/936).
pub struct SnapshotWithDBArc {
    // We hold an Arc to the database to ensure the snapshot cannot outlive it.
    pub(crate) db: Arc<RocksDBStorageInner>,
    // The Database needs to outlive the snapshot, and be created by the supplied `db`.
    inner: *const ffi::rocksdb_snapshot_t,
}

#[allow(dead_code)]
impl SnapshotWithDBArc {
    /// Iterates a column using this snapshot, retaining its borrow until the iterator is dropped.
    /// Raw snapshot read options never escape independently of the snapshot's lifetime.
    pub(crate) fn iterator_cf(
        &self,
        column: Column,
        mut readopts: ReadOptions,
        mode: IteratorMode<'_>,
    ) -> DBIterator<'_> {
        // SAFETY: the column and snapshot belong to this DB. The returned iterator borrows
        // self, keeping both the snapshot and its backing DB alive while RocksDB uses them.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        DBIterator::new_cf(&self.db.db, &self.db.get_column(column), readopts, mode)
    }

    // Keep raw options inside this private callback while self keeps the snapshot alive.
    fn readopts_with_raw_snapshot<R>(&self, mut readopts: ReadOptions, f: impl FnOnce(&ReadOptions) -> R) -> R {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }

        f(&readopts)
    }

    /// Creates a new `SnapshotWithDBArc` of the database `db`.
    pub(crate) fn new(db: Arc<RocksDBStorageInner>) -> Self {
        // SAFETY: db is live and its Arc is retained until after the snapshot is released.
        let snapshot = unsafe { db.db.create_snapshot() };
        Self { db, inner: snapshot }
    }

    /// Returns the bytes associated with a key value with default read options.
    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Error> {
        let readopts = ReadOptions::default();
        self.get_opt(key, readopts)
    }

    /// Returns the bytes associated with a key value and given column family with default read
    /// options.
    pub fn get_cf<K: AsRef<[u8]>>(&self, cf: &impl AsColumnFamilyRef, key: K) -> Result<Option<Vec<u8>>, Error> {
        let readopts = ReadOptions::default();
        self.get_cf_opt(cf, key.as_ref(), readopts)
    }

    /// Returns the bytes associated with a key value and given read options.
    pub fn get_opt<K: AsRef<[u8]>>(&self, key: K, readopts: ReadOptions) -> Result<Option<Vec<u8>>, Error> {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.get_opt(key.as_ref(), readopts))
    }

    /// Returns the bytes associated with a key value, given column family and read options.
    pub fn get_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        readopts: ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.get_cf_opt(cf, key.as_ref(), readopts))
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// leverages default options.
    pub fn get_pinned<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        let readopts = ReadOptions::default();
        self.get_pinned_opt(key, readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_cf_opt but
    /// leverages default options.
    pub fn get_pinned_cf<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
    ) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        let readopts = ReadOptions::default();
        self.get_pinned_cf_opt(cf, key.as_ref(), readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy.
    pub fn get_pinned_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.get_pinned_opt(key.as_ref(), readopts))
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// allows specifying ColumnFamily.
    pub fn get_pinned_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice<'_>>, Error> {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.get_pinned_cf_opt(cf, key.as_ref(), readopts))
    }

    /// Returns the bytes associated with the given key values and default read options.
    pub fn multi_get<K: AsRef<[u8]>, I>(&self, keys: I) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        I: IntoIterator<Item = K>,
    {
        let readopts = ReadOptions::default();
        self.multi_get_opt(keys, readopts)
    }

    /// Returns the bytes associated with the given key values and default read options.
    pub fn multi_get_cf<'b, K, I, W>(&self, keys_cf: I) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'b W, K)>,
        W: AsColumnFamilyRef + 'b,
    {
        let readopts = ReadOptions::default();
        self.multi_get_cf_opt(keys_cf, readopts)
    }

    /// Returns the bytes associated with the given key values and given read options.
    pub fn multi_get_opt<K, I>(&self, keys: I, readopts: ReadOptions) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = K>,
    {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.multi_get_opt(keys, readopts))
    }

    /// Returns the bytes associated with the given key values, given column family and read options.
    pub fn multi_get_cf_opt<'b, K, I, W>(
        &self,
        keys_cf: I,
        readopts: ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'b W, K)>,
        W: AsColumnFamilyRef + 'b,
    {
        self.readopts_with_raw_snapshot(readopts, |readopts| self.db.db.multi_get_cf_opt(keys_cf, readopts))
    }
}

impl Drop for SnapshotWithDBArc {
    fn drop(&mut self) {
        // SAFETY: this handle came from this DB and is released exactly once, before its Arc drops.
        unsafe {
            self.db.db.release_snapshot(self.inner);
        }
    }
}

// SAFETY: RocksDB snapshots can be used across threads; the Arc keeps the owning DB alive.
unsafe impl Send for SnapshotWithDBArc {}
// SAFETY: shared access only reads the immutable snapshot; releasing it requires exclusive Drop.
unsafe impl Sync for SnapshotWithDBArc {}
