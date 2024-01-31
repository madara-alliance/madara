use std::marker::PhantomData;
use std::sync::Arc;

use bonsai_trie::bonsai_database::DatabaseKey;
use bonsai_trie::BonsaiDatabase;
use sp_database::{Change, Database, Transaction};
use sp_runtime::traits::Block as BlockT;

use crate::{DbError, DbHash};

pub struct BonsaiDb<B: BlockT> {
    pub(crate) db: Arc<dyn Database<DbHash>>,
    pub(crate) _marker: PhantomData<B>,
}

impl<B: BlockT> BonsaiDb<B> {
    fn apply_changes(&self, changes: Vec<Change<DbHash>>) -> Result<(), DbError> {
        let mut transaction = Transaction::new();
        for change in changes {
            match change {
                Change::Set(column, key, value) => transaction.set(column, &key, &value),
                Change::Remove(column, key) => transaction.remove(column, &key),
                _ => unimplemented!(),
            }
        }
        self.db.commit(transaction).map_err(DbError::from)
    }
}

impl<B: BlockT> BonsaiDatabase for BonsaiDb<B> {
    type Batch = Vec<Change<DbHash>>;
    type DatabaseError = DbError;

    fn create_batch(&self) -> Self::Batch {
        Vec::new()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        match self.db.get(crate::columns::BONSAI, &key.as_slice()) {
            Some(raw) => Ok(Some(raw)),
            None => Ok(None),
        }
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        let result = self.db.get(crate::columns::BONSAI, key.as_slice());
        match result {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        // let mut result = Vec::new();
        // let prefix_bytes = prefix.as_slice();
        // for (key, value) in self.db. entries(crate::columns::BONSAI) {
        //     if key.starts_with(prefix_bytes) {
        //         result.push((key, value));
        //     }
        // }
        Ok(vec![])
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        match batch {
            Some(b) => {
                b.push(Change::Set(crate::columns::BONSAI, key.as_slice().to_vec(), value.to_vec()));
                Ok(None)
            }
            None => {
                let result = self.db.get(crate::columns::BONSAI, key.as_slice());
                match result {
                    Some(old_value) => {
                        let mut transaction = Transaction::new();
                        transaction.set(crate::columns::BONSAI, key.as_slice(), value);
                        self.db.commit(transaction)?;
                        Ok(Some(old_value))
                    }
                    None => {
                        let mut transaction = Transaction::new();
                        transaction.set(crate::columns::BONSAI, key.as_slice(), value);
                        self.db.commit(transaction)?;
                        Ok(None)
                    }
                }
            }
        }
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        match batch {
            Some(b) => {
                b.push(Change::Remove(crate::columns::BONSAI, key.as_slice().to_vec()));
                Ok(None)
            }
            None => {
                let result = self.db.get(crate::columns::BONSAI, key.as_slice());
                match result {
                    Some(old_value) => {
                        let mut transaction = Transaction::new();
                        transaction.remove(crate::columns::BONSAI, key.as_slice());
                        self.db.commit(transaction)?;
                        Ok(Some(old_value))
                    }
                    None => Ok(None),
                }
            }
        }
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        // let prefix_bytes = prefix.as_slice();
        // for (key, _) in self.db.iter(crate::columns::BONSAI) {
        //     if key.starts_with(prefix_bytes) {
        //         let mut transaction = Transaction::new();
        //         transaction.remove(crate::columns::BONSAI, &key);
        //         self.db.commit(transaction)?;
        //     }
        // }
        Ok(())
    }

    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        self.apply_changes(batch)
    }
}
