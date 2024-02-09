use std::marker::PhantomData;
use std::sync::Arc;

use bonsai_trie::id::Id;
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, DatabaseKey};
use kvdb::{DBTransaction, KeyValueDB};
use sp_runtime::traits::Block as BlockT;

use crate::error::BonsaiDbError;

/// Represents a Bonsai database instance parameterized by a block type.
pub struct BonsaiDb<B: BlockT> {
    /// Database interface for key-value operations.
    pub(crate) db: Arc<dyn KeyValueDB>,
    /// PhantomData to mark the block type used.
    pub(crate) _marker: PhantomData<B>,
}

impl<B: BlockT> BonsaiDatabase for &BonsaiDb<B> {
    type Batch = DBTransaction;
    type DatabaseError = BonsaiDbError;

    /// Creates a new database transaction batch.
    fn create_batch(&self) -> Self::Batch {
        DBTransaction::new()
    }

    /// Retrieves a value by its database key.
    fn get(&self, key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        self.db.get(column, key_slice).map_err(Into::into)
    }

    /// Inserts a key-value pair into the database, optionally within a provided batch.
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

    /// Checks if a key exists in the database.
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let key_slice = key.as_slice();
        self.db.has_key(column, key_slice).map_err(Into::into)
    }

    /// Retrieves all key-value pairs starting with a given prefix.
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let prefix_slice = prefix.as_slice();
        let mut result = Vec::new();

        for pair in self.db.iter_with_prefix(column, prefix_slice) {
            let pair = pair.map_err(|e| BonsaiDbError::from(e))?;
            result.push((pair.0.into_vec(), pair.1));
        }

        Ok(result)
    }

    /// Removes a key-value pair from the database, optionally within a provided batch.
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

    /// Removes all key-value pairs starting with a given prefix.
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let column = crate::columns::BONSAI;
        let prefix_slice = prefix.as_slice();
        let mut transaction = self.create_batch();
        transaction.delete_prefix(column, prefix_slice);
        self.db.write(transaction).map_err(Into::into)
    }

    /// Writes a batch of changes to the database.
    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        self.db.write(batch).map_err(Into::into)
    }
}

/// A wrapper around a database transaction.
pub struct TransactionWrapper {
    /// The underlying database transaction.
    _transaction: DBTransaction,
}

/// This implementation is a stub to mute BonsaiPersistentDatabase that is currently not needed.
impl BonsaiDatabase for TransactionWrapper {
    type Batch = DBTransaction;
    type DatabaseError = BonsaiDbError;

    /// Creates a new database transaction batch.
    fn create_batch(&self) -> Self::Batch {
        DBTransaction::new()
    }

    /// Retrieves a value by its database key.
    fn get(&self, _key: &DatabaseKey) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        // Simulate database access without performing real operations
        Ok(None)
    }

    /// Inserts a key-value pair into the database.
    fn insert(
        &mut self,
        _key: &DatabaseKey,
        _value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        Ok(None)
    }

    /// Checks if a key exists in the database.
    fn contains(&self, _key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        Ok(false)
    }

    /// Retrieves all key-value pairs starting with a given prefix.
    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Self::DatabaseError> {
        Ok(Vec::new())
    }

    /// Removes a key-value pair from the database.
    fn remove(
        &mut self,
        _key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<Vec<u8>>, Self::DatabaseError> {
        Ok(None)
    }

    /// Removes all key-value pairs starting with a given prefix.
    fn remove_by_prefix(&mut self, _prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        Ok(())
    }

    /// Writes a batch of changes to the database.
    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(())
    }
}

/// This implementation is a stub to mute any error but is is currently not used.
impl<B: BlockT, ID: Id> BonsaiPersistentDatabase<ID> for &BonsaiDb<B> {
    type Transaction = TransactionWrapper;
    type DatabaseError = BonsaiDbError;

    /// Snapshots the current database state, associated with an ID.
    fn snapshot(&mut self, _id: ID) {}

    /// Begins a new transaction associated with a snapshot ID.
    fn transaction(&self, _id: ID) -> Option<Self::Transaction> {
        None
    }

    /// Merges a completed transaction into the persistent database.
    fn merge(&mut self, _transaction: Self::Transaction) -> Result<(), Self::DatabaseError> {
        Ok(())
    }
}
