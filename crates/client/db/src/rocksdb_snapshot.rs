use librocksdb_sys as ffi;
use rocksdb::{AsColumnFamilyRef, DBAccess, DBPinnableSlice, Error, ReadOptions};
use std::sync::Arc;

/// A copy of [`rocksdb::SnapshotWithThreadMode`] with an Arc<DB> instead of an &'_ DB reference
pub struct SnapshotWithDBArc<D: DBAccess> {
    db: Arc<D>,
    // The Database needs to outlive the snapshot, and be created by the supplied `db`.
    inner: *const ffi::rocksdb_snapshot_t,
}

#[allow(dead_code)]
impl<D: DBAccess> SnapshotWithDBArc<D> {
    /// Creates a new `SnapshotWithDBArc` of the database `db`.
    pub fn new(db: Arc<D>) -> Self {
        let snapshot = unsafe { db.create_snapshot() };
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
    pub fn get_opt<K: AsRef<[u8]>>(&self, key: K, mut readopts: ReadOptions) -> Result<Option<Vec<u8>>, Error> {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.get_opt(key.as_ref(), &readopts)
    }

    /// Returns the bytes associated with a key value, given column family and read options.
    pub fn get_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<Vec<u8>>, Error> {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.get_cf_opt(cf, key.as_ref(), &readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// leverages default options.
    pub fn get_pinned<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<DBPinnableSlice>, Error> {
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
    ) -> Result<Option<DBPinnableSlice>, Error> {
        let readopts = ReadOptions::default();
        self.get_pinned_cf_opt(cf, key.as_ref(), readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy.
    pub fn get_pinned_opt<K: AsRef<[u8]>>(
        &self,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice>, Error> {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.get_pinned_opt(key.as_ref(), &readopts)
    }

    /// Return the value associated with a key using RocksDB's PinnableSlice
    /// so as to avoid unnecessary memory copy. Similar to get_pinned_opt but
    /// allows specifying ColumnFamily.
    pub fn get_pinned_cf_opt<K: AsRef<[u8]>>(
        &self,
        cf: &impl AsColumnFamilyRef,
        key: K,
        mut readopts: ReadOptions,
    ) -> Result<Option<DBPinnableSlice>, Error> {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.get_pinned_cf_opt(cf, key.as_ref(), &readopts)
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
    pub fn multi_get_opt<K, I>(&self, keys: I, mut readopts: ReadOptions) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = K>,
    {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.multi_get_opt(keys, &readopts)
    }

    /// Returns the bytes associated with the given key values, given column family and read options.
    pub fn multi_get_cf_opt<'b, K, I, W>(
        &self,
        keys_cf: I,
        mut readopts: ReadOptions,
    ) -> Vec<Result<Option<Vec<u8>>, Error>>
    where
        K: AsRef<[u8]>,
        I: IntoIterator<Item = (&'b W, K)>,
        W: AsColumnFamilyRef + 'b,
    {
        // Safety: the snapshot originates from the `db`, and it is not dropped during the lifetime of the `readopts` variable.
        unsafe {
            readopts.set_raw_snapshot(self.inner);
        }
        self.db.multi_get_cf_opt(keys_cf, &readopts)
    }
}

impl<D: DBAccess> Drop for SnapshotWithDBArc<D> {
    fn drop(&mut self) {
        unsafe {
            self.db.release_snapshot(self.inner);
        }
    }
}

/// `Send` and `Sync` implementations for `SnapshotWithThreadMode` are safe, because `SnapshotWithThreadMode` is
/// immutable and can be safely shared between threads.
unsafe impl<D: DBAccess> Send for SnapshotWithDBArc<D> {}
unsafe impl<D: DBAccess> Sync for SnapshotWithDBArc<D> {}