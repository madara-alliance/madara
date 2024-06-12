use std::ops::Deref;
use std::{marker::PhantomData, sync::Arc};

use crossbeam_skiplist::SkipMap;
use rayon::prelude::ParallelIterator;
use rayon::slice::ParallelSlice;
use rocksdb::{IteratorMode, ReadOptions, WriteBatchWithTransaction};
use thiserror::Error;

use super::{codec, DeoxysStorageError, StorageView, StorageViewMut};
use crate::{Column, DatabaseExt, DB};

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("rocksdb error: {0}")]
    RocksDBError(#[from] rocksdb::Error),
    #[error("db number format error")]
    NumberFormat,
    #[error("value codec error: {0}")]
    Codec(#[from] codec::Error),
}

pub struct History<K: Deref<Target = [u8]>, T> {
    column: Column,
    prefix: K,
    _boo: PhantomData<T>,
}

impl<K: Deref<Target = [u8]>, T: codec::Decode + codec::Encode> History<K, T> {
    pub fn open(column: Column, prefix: K) -> Self {
        Self { column, prefix, _boo: PhantomData }
    }

    pub fn get_at(&self, db: &DB, block_n: u64) -> Result<Option<(u64, T)>, HistoryError> {
        let block_n = u32::try_from(block_n).map_err(|_| HistoryError::NumberFormat)?;
        let start_at = [self.prefix.deref(), &block_n.to_be_bytes() as &[u8]].concat();

        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        // options.set_iterate_range(PrefixRange(&prefix as &[u8]));
        let mode = IteratorMode::From(&start_at, rocksdb::Direction::Reverse);
        let mut iter = db.iterator_cf_opt(&db.get_column(self.column), options, mode);

        match iter.next() {
            Some(res) => {
                let (k, v) = res?;
                assert!(k.starts_with(self.prefix.deref()));
                let block_n: [u8; 4] =
                    k[self.prefix.deref().len()..].try_into().map_err(|_| HistoryError::NumberFormat)?;
                let block_n = u32::from_be_bytes(block_n);

                let v = T::decode(v.deref())?;

                Ok(Some((block_n.into(), v)))
            }
            None => Ok(None),
        }
    }

    pub fn get_last(&self, db: &DB) -> Result<Option<(u64, T)>, HistoryError> {
        self.get_at(db, u32::MAX.into())
    }

    pub fn put(
        &self,
        db: &DB,
        write_batch: &mut WriteBatchWithTransaction<true>,
        block_n: u64,
        value: T,
    ) -> Result<(), HistoryError> {
        let block_n = u32::try_from(block_n).map_err(|_| HistoryError::NumberFormat)?;
        let key = [self.prefix.deref(), &block_n.to_be_bytes() as &[u8]].concat();
        write_batch.put_cf(&db.get_column(self.column), key, value.encode()?);
        Ok(())
    }
}

// View/ViewMut storage handler implementations

pub trait AsHistoryView {
    type Key: Ord + Sync + Send + Clone + 'static;
    type KeyBin: Deref<Target = [u8]> + From<Self::Key>;
    type T: codec::Decode + codec::Encode + Sync + Send + Clone + 'static;

    fn column() -> Column;
}

pub struct HistoryView<R: AsHistoryView>(Arc<DB>, PhantomData<R>);
impl<R: AsHistoryView> HistoryView<R> {
    pub(crate) fn new(backend: Arc<DB>) -> Self {
        Self(backend, PhantomData)
    }
}

impl<R: AsHistoryView> StorageView for HistoryView<R> {
    type KEY = R::Key;
    type VALUE = R::T;

    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let db = &self.0;
        let key = R::KeyBin::from(key.clone());
        let history = History::open(R::column(), key);

        let got = history.get_last(db)?;

        Ok(got.map(|(_block, v)| v))
    }

    fn contains(&self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.get(key).map(|a| a.is_some())
    }
}

impl<R: AsHistoryView> HistoryView<R> {
    pub fn get_at(&self, key: &R::Key, block_number: u64) -> Result<Option<R::T>, DeoxysStorageError> {
        let db = &self.0;
        let key = R::KeyBin::from(key.clone());
        let history = History::open(R::column(), key);

        let got = history.get_at(db, block_number)?;

        Ok(got.map(|(_block, v)| v))
    }
}

#[derive(Debug)]
pub struct HistoryViewMut<R: AsHistoryView>(Arc<DB>, SkipMap<R::Key, R::T>);
impl<R: AsHistoryView> HistoryViewMut<R> {
    pub(crate) fn new(backend: Arc<DB>) -> Self {
        Self(backend, Default::default())
    }
}

impl<R: AsHistoryView> StorageViewMut for HistoryViewMut<R> {
    type KEY = R::Key;
    type VALUE = R::T;

    /// Insert data into storage.
    ///
    /// * `key`: identifier used to inser data.
    /// * `value`: encodable data to save to the database.
    fn insert(&self, key: Self::KEY, value: Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.1.insert(key, value);
        Ok(())
    }

    /// Applies all changes up to this point.
    ///
    /// * `block_number`: point in the chain at which to apply the new changes. Must be incremental
    fn commit(self, block_number: u64) -> Result<(), DeoxysStorageError> {
        let db = &self.0;

        let as_vec = self.1.into_iter().collect::<Vec<_>>(); // todo: use proper datastructure that supports rayon

        as_vec.deref().par_chunks(1024).try_for_each(|chunk| {
            let mut batch = WriteBatchWithTransaction::<true>::default();
            for (key, v) in chunk {
                let key = R::KeyBin::from(key.clone());

                History::open(R::column(), key).put(db, &mut batch, block_number, v.clone())?;
            }
            db.write(batch)?;
            Ok::<_, HistoryError>(())
        })?;

        Ok(())
    }
}

impl<R: AsHistoryView> StorageView for HistoryViewMut<R> {
    type KEY = R::Key;
    type VALUE = R::T;

    fn get(&self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        if let Some(entry) = self.1.get(key) {
            return Ok(Some(entry.value().clone()));
        }
        HistoryView::<R>::new(Arc::clone(&self.0)).get(key)
    }

    fn contains(&self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.get(key).map(|a| a.is_some())
    }
}
