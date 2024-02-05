use std::marker::PhantomData;
use std::sync::Arc;

use bonsai_trie::bonsai_database::DatabaseKey;
use bonsai_trie::BonsaiDatabase;
use kvdb::{DBTransaction, KeyValueDB};
use sp_runtime::traits::Block as BlockT;

use crate::error::BonsaiDbError;

pub struct BonsaiDb<B: BlockT> {
    pub(crate) db: Arc<dyn KeyValueDB>,
    pub(crate) _marker: PhantomData<B>,
}

impl<B: BlockT> BonsaiDatabase for BonsaiDb<B> {
    type Batch = DBTransaction;
    type DatabaseError = BonsaiDbError;

    fn create_batch(&self) -> Self::Batch {
        DBTransaction::new()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        self.db.get(column, key_slice).map_err(Into::into)
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        let previous_value = self.db.get(column, key_slice)?;

        if let Some(batch) = batch {
            batch.put(column, key_slice, value);
        } else {
            let mut transaction = self.create_batch();
            transaction.put(column, key_slice, value);
            self.db.write(transaction)?;
        }

        Ok(previous_value)
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        self.db.has_key(column, key_slice).map_err(Into::into)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let prefix_slice = prefix.as_slice();
        let mut result = Vec::new();

        // Iterate over the database entries with the given prefix
        for pair in self.db.iter_with_prefix(column, prefix_slice) {
            let pair = pair.map_err(|e| BonsaiDbError::from(e))?;
            result.push((pair.0.into_vec(), pair.1));
        }

        Ok(result)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        let previous_value = self.db.get(column, key_slice)?;

        if let Some(batch) = batch {
            batch.delete(column, key_slice);
        } else {
            let mut transaction = self.create_batch();
            transaction.delete(column, key_slice);
            self.db.write(transaction)?;
        }

        Ok(previous_value)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let prefix_slice = prefix.as_slice();
        let mut transaction = self.create_batch();
        transaction.delete_prefix(column, prefix_slice);
        self.db.write(transaction).map_err(Into::into)
    }

    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        self.db.write(batch).map_err(Into::into)
    }
}
