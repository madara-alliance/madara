use crate::rocksdb::DB;
use rocksdb::{AsColumnFamilyRef, DBRawIteratorWithThreadMode, Direction, IteratorMode, ReadOptions};

pub struct DBIterator<'a> {
    raw: DBRawIteratorWithThreadMode<'a, DB>,
    should_advance: bool,
    direction: Direction,
    done: bool,
}
impl<'a> DBIterator<'a> {
    pub fn new_cf(db: &'a DB, cf: &impl AsColumnFamilyRef, readopts: ReadOptions, mode: IteratorMode) -> Self {
        Self::from_raw(db.raw_iterator_cf_opt(cf, readopts), mode)
    }

    fn from_raw(raw: DBRawIteratorWithThreadMode<'a, DB>, mode: IteratorMode) -> Self {
        let mut rv = DBIterator {
            raw,
            direction: Direction::Forward, // blown away by set_mode()
            done: false,
            should_advance: false,
        };
        rv.set_mode(mode);
        rv
    }

    pub fn set_mode(&mut self, mode: IteratorMode) {
        self.done = false;
        self.should_advance = false;
        self.direction = match mode {
            IteratorMode::Start => {
                self.raw.seek_to_first();
                Direction::Forward
            }
            IteratorMode::End => {
                self.raw.seek_to_last();
                Direction::Reverse
            }
            IteratorMode::From(key, Direction::Forward) => {
                self.raw.seek(key);
                Direction::Forward
            }
            IteratorMode::From(key, Direction::Reverse) => {
                self.raw.seek_for_prev(key);
                Direction::Reverse
            }
        };
    }

    fn pre_advance(&mut self) -> bool {
        if self.should_advance {
            match self.direction {
                Direction::Forward => self.raw.next(),
                Direction::Reverse => self.raw.prev(),
            }
        }
        self.should_advance = true;
        true
    }

    /// Returns false when exhausted. (true = "valid", meaning .raw.item() .raw.key() .raw.value() can be used.)
    pub fn next(&mut self) -> Result<bool, rocksdb::Error> {
        if self.done {
            return Ok(false);
        }
        self.pre_advance();
        if self.raw.valid() {
            Ok(true) // valid
        } else {
            self.done = true;
            self.raw.status().map(|_| false) // exhausted
        }
    }

    pub fn into_iter_keys<R, F: FnMut(&[u8]) -> R>(self, map: F) -> DBKeyIterator<'a, R, F> {
        DBKeyIterator { iter: self, map }
    }
    pub fn into_iter_values<R, F: FnMut(&[u8]) -> R>(self, map: F) -> DBValueIterator<'a, R, F> {
        DBValueIterator { iter: self, map }
    }
    pub fn into_iter_items<R, F: FnMut((&[u8], &[u8])) -> R>(self, map: F) -> DBItemsIterator<'a, R, F> {
        DBItemsIterator { iter: self, map }
    }
}

pub struct DBItemsIterator<'a, R, F: FnMut((&[u8], &[u8])) -> R + 'a> {
    iter: DBIterator<'a>,
    map: F,
}
impl<'a, R, F: FnMut((&[u8], &[u8])) -> R + 'a> Iterator for DBItemsIterator<'a, R, F> {
    type Item = Result<R, rocksdb::Error>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            Ok(true) => Some(Ok((self.map)(self.iter.raw.item().expect("Valid iterator should have an item ready")))),
            // Empty
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        }
    }
}
impl<'a, R, F: FnMut((&[u8], &[u8])) -> R + 'a> std::iter::FusedIterator for DBItemsIterator<'a, R, F> {}

pub struct DBValueIterator<'a, R, F: FnMut(&[u8]) -> R + 'a> {
    iter: DBIterator<'a>,
    map: F,
}
impl<'a, R, F: FnMut(&[u8]) -> R + 'a> Iterator for DBValueIterator<'a, R, F> {
    type Item = Result<R, rocksdb::Error>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            Ok(true) => Some(Ok((self.map)(self.iter.raw.value().expect("Valid iterator should have an item ready")))),
            // Empty
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        }
    }
}
impl<'a, R, F: FnMut(&[u8]) -> R + 'a> std::iter::FusedIterator for DBValueIterator<'a, R, F> {}

pub struct DBKeyIterator<'a, R, F: FnMut(&[u8]) -> R + 'a> {
    iter: DBIterator<'a>,
    map: F,
}
impl<'a, R, F: FnMut(&[u8]) -> R + 'a> Iterator for DBKeyIterator<'a, R, F> {
    type Item = Result<R, rocksdb::Error>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            Ok(true) => Some(Ok((self.map)(self.iter.raw.key().expect("Valid iterator should have an item ready")))),
            // Empty
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        }
    }
}
impl<'a, R, F: FnMut(&[u8]) -> R + 'a> std::iter::FusedIterator for DBKeyIterator<'a, R, F> {}
