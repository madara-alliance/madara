use kvdb::{DBTransaction, KeyValueDB};
use sp_database::error::DatabaseError;
use sp_database::{Change, ColumnId, Database, Transaction};

#[repr(transparent)]
pub struct DbAdapter(kvdb_rocksdb::Database);

fn handle_err<T>(result: std::io::Result<T>) -> T {
    match result {
        Ok(r) => r,
        Err(e) => {
            panic!("Critical database error: {:?}", e);
        }
    }
}

impl DbAdapter {
    // Returns counter key and counter value if it exists.
    fn read_counter(&self, col: ColumnId, key: &[u8]) -> Result<(Vec<u8>, Option<u32>), DatabaseError> {
        // Add a key suffix for the counter
        let mut counter_key = key.to_vec();
        counter_key.push(0);
        Ok(match self.0.get(col, &counter_key).map_err(|e| DatabaseError(Box::new(e)))? {
            Some(data) => {
                let mut counter_data = [0; 4];
                if data.len() != 4 {
                    return Err(DatabaseError(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Unexpected counter len {}", data.len()),
                    ))));
                }
                counter_data.copy_from_slice(&data);
                let counter = u32::from_le_bytes(counter_data);
                (counter_key, Some(counter))
            }
            None => (counter_key, None),
        })
    }
}

impl<H: Clone + AsRef<[u8]>> Database<H> for DbAdapter {
    fn commit(&self, transaction: Transaction<H>) -> Result<(), DatabaseError> {
        let mut tx = DBTransaction::new();
        for change in transaction.0.into_iter() {
            match change {
                Change::Set(col, key, value) => tx.put_vec(col, &key, value),
                Change::Remove(col, key) => tx.delete(col, &key),
                Change::Store(col, key, value) => match self.read_counter(col, key.as_ref())? {
                    (counter_key, Some(mut counter)) => {
                        counter += 1;
                        tx.put(col, &counter_key, &counter.to_le_bytes());
                    }
                    (counter_key, None) => {
                        let d = 1u32.to_le_bytes();
                        tx.put(col, &counter_key, &d);
                        tx.put_vec(col, key.as_ref(), value);
                    }
                },
                Change::Reference(col, key) => {
                    if let (counter_key, Some(mut counter)) = self.read_counter(col, key.as_ref())? {
                        counter += 1;
                        tx.put(col, &counter_key, &counter.to_le_bytes());
                    }
                }
                Change::Release(col, key) => {
                    if let (counter_key, Some(mut counter)) = self.read_counter(col, key.as_ref())? {
                        counter -= 1;
                        if counter == 0 {
                            tx.delete(col, &counter_key);
                            tx.delete(col, key.as_ref());
                        } else {
                            tx.put(col, &counter_key, &counter.to_le_bytes());
                        }
                    }
                }
            }
        }
        self.0.write(tx).map_err(|e| DatabaseError(Box::new(e)))
    }

    fn get(&self, col: ColumnId, key: &[u8]) -> Option<Vec<u8>> {
        handle_err(self.0.get(col, key))
    }

    fn contains(&self, col: ColumnId, key: &[u8]) -> bool {
        handle_err(self.0.has_key(col, key))
    }
}
