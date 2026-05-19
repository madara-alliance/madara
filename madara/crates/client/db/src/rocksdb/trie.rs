use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::{SnapshotRef, Snapshots};
use crate::rocksdb::{RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction};
use bonsai_trie::id::Id;
use bonsai_trie::{
    BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, BonsaiStorageConfig, ByteVec, DBError, DatabaseKey,
};
use rocksdb::{Direction, IteratorMode};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub const BONSAI_CONTRACT_FLAT_COLUMN: Column = Column::new("bonsai_contract_flat").set_point_lookup();
pub const BONSAI_CONTRACT_TRIE_COLUMN: Column = Column::new("bonsai_contract_trie").set_point_lookup();
pub const BONSAI_CONTRACT_LOG_COLUMN: Column = Column::new("bonsai_contract_log");
pub const BONSAI_CONTRACT_STORAGE_FLAT_COLUMN: Column = Column::new("bonsai_contract_storage_flat").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_TRIE_COLUMN: Column = Column::new("bonsai_contract_storage_trie").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_LOG_COLUMN: Column = Column::new("bonsai_contract_storage_log");
pub const BONSAI_CLASS_FLAT_COLUMN: Column = Column::new("bonsai_class_flat").set_point_lookup();
pub const BONSAI_CLASS_TRIE_COLUMN: Column = Column::new("bonsai_class_trie").set_point_lookup();
pub const BONSAI_CLASS_LOG_COLUMN: Column = Column::new("bonsai_class_log");

pub type GlobalTrie<H> = BonsaiStorage<BasicId, BonsaiDB, H>;

pub use bonsai_trie::id::BasicId;
pub use bonsai_trie::ProofNode;

/// Wrapper because bonsai requires a special DBError trait implementation.
/// TODO: Remove that upstream in bonsai-trie, this is dumb.
#[derive(thiserror::Error, Debug)]
pub enum TrieError {
    #[error(transparent)]
    RocksDb(#[from] rocksdb::Error),
    #[error("Invalid bonsai trie-log format: {0}")]
    InvalidTrieLogFormat(String),
}
impl DBError for TrieError {}

/// Wrapper because bonsai's error type does not implement [std::error::Error].
/// TODO: Fix that upstream in bonsai-trie, this is seriously dumb.
#[derive(thiserror::Error, Debug)]
#[error("Global trie error: {0:#}")]
pub struct WrappedBonsaiError(pub bonsai_trie::BonsaiStorageError<TrieError>);

impl RocksDBStorage {
    fn get_bonsai<H: StarkHash + Send + Sync>(
        &self,
        column_mapping: DatabaseKeyMapping,
    ) -> BonsaiStorage<BasicId, BonsaiDB, H> {
        BonsaiStorage::new(
            BonsaiDB { backend: self.inner.clone(), column_mapping, snapshots: self.snapshots.clone() },
            BonsaiStorageConfig {
                max_saved_trie_logs: self.inner.config.max_saved_trie_logs,
                max_saved_snapshots: self.inner.config.max_kept_snapshots,
                snapshot_interval: self.inner.config.snapshot_interval,
            },
            // Every global tree has keys of 251 bits.
            251,
        )
    }
    pub fn contract_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_TRIE_COLUMN,
            log: BONSAI_CONTRACT_LOG_COLUMN,
        })
    }
    pub fn contract_storage_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        })
    }
    pub fn class_trie(&self) -> GlobalTrie<Poseidon> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CLASS_FLAT_COLUMN,
            trie: BONSAI_CLASS_TRIE_COLUMN,
            log: BONSAI_CLASS_LOG_COLUMN,
        })
    }
}

#[derive(Clone, Debug)]
struct DatabaseKeyMapping {
    flat: Column,
    trie: Column,
    log: Column,
}

impl DatabaseKeyMapping {
    pub(crate) fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }
}

pub struct BonsaiDB {
    backend: Arc<RocksDBStorageInner>,
    snapshots: Arc<Snapshots>,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
}

impl fmt::Debug for BonsaiDB {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BonsaiDB {{}}")
    }
}

impl BonsaiDatabase for BonsaiDB {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    #[tracing::instrument(skip(self, key))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        Ok(self.backend.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix))]
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        tracing::trace!("Getting by prefix from RocksDB: {:?}", prefix);
        let handle = self.backend.get_column(self.column_mapping.map(prefix).clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
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

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        Ok(self.backend.db.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    #[tracing::instrument(skip(self, key, value, batch))]
    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());

        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.backend.db.put_cf_opt(&handle, key.as_slice(), value, &self.backend.writeopts)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, key, batch))]
    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.backend.db.delete_cf_opt(&handle, key.as_slice(), &self.backend.writeopts)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix))]
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.backend.get_column(self.column_mapping.map(prefix).clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
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

    #[tracing::instrument(skip(self, batch))]
    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(self.backend.db.write_opt(batch, &self.backend.writeopts)?)
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

// These constants mirror the trie-log encoding in
// `bonsai-trie` commit `13e8f7b`:
// `/src/changes.rs::{KEY_SEPARATOR, NEW_VALUE, OLD_VALUE}` and the key layout
// produced by `key_old_value` / `key_new_value`.
const TRIE_KEY_KIND: u8 = 0;
const FLAT_KEY_KIND: u8 = 1;
const TRIE_LOG_KEY_SEPARATOR: u8 = 0x00;
const TRIE_LOG_NEW_VALUE: u8 = 0x00;
const TRIE_LOG_OLD_VALUE: u8 = 0x01;

#[derive(Default)]
struct TrieLogChange {
    old_value: Option<ByteVec>,
    new_value: Option<ByteVec>,
}

fn database_key_from_parts<'a>(key_kind: u8, key: &'a [u8]) -> Result<DatabaseKey<'a>, TrieError> {
    match key_kind {
        TRIE_KEY_KIND => Ok(DatabaseKey::Trie(key)),
        FLAT_KEY_KIND => Ok(DatabaseKey::Flat(key)),
        _ => Err(TrieError::InvalidTrieLogFormat(format!("unsupported key kind byte {key_kind}"))),
    }
}

/// The backing database for a bonsai storage view. This is used
/// to implement historical access (for storage proofs), by applying
/// changes from the trie-log without modifying the real database.
///
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

impl BonsaiTransaction {
    fn apply_change(&mut self, key_kind: u8, key: &[u8], value: Option<&[u8]>) -> Result<(), TrieError> {
        let key = database_key_from_parts(key_kind, key)?;
        match value {
            Some(value) => {
                self.insert(&key, value, None)?;
            }
            None => {
                self.remove(&key, None)?;
            }
        }
        Ok(())
    }
}

// TODO: a lot of this is not really used yet, this whole abstraction does not really make sense anyway, this needs to be modified
// upstream in bonsai-trie
impl BonsaiDatabase for BonsaiTransaction {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Default::default()
    }

    #[tracing::instrument(skip(self, key))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        if let Some(val) = self.changed.get(&to_changed_key(key)) {
            return Ok(val.clone());
        }
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        unreachable!("unused for now")
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        if let Some(val) = self.changed.get(&to_changed_key(key)) {
            return Ok(val.is_some());
        }
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
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

impl BonsaiPersistentDatabase<BasicId> for BonsaiDB {
    type Transaction<'a>
        = BonsaiTransaction
    where
        Self: 'a;
    type DatabaseError = TrieError;

    /// this is called upstream, but we ignore it for now because we create the snapshot in [`crate::MadaraBackend::store_block`]
    #[tracing::instrument(skip(self))]
    fn snapshot(&mut self, id: BasicId) {}

    #[tracing::instrument(skip(self))]
    fn transaction(&self, requested_id: BasicId) -> Option<(BasicId, Self::Transaction<'_>)> {
        tracing::trace!("Generating RocksDB transaction");
        let requested_id_u64 = requested_id.as_u64();
        let (snapshot_id, snapshot) = self.snapshots.get_closest(requested_id_u64);

        tracing::debug!("Snapshot for requested block_id={requested_id:?} => got block_id={snapshot_id:?}");

        let snapshot_id = snapshot_id?;
        let mut txn =
            BonsaiTransaction { snapshot, column_mapping: self.column_mapping.clone(), changed: Default::default() };

        if let Err(error) = self.replay_trie_logs_into_transaction(&mut txn, snapshot_id, requested_id_u64) {
            tracing::error!(
                ?error,
                requested_id = requested_id_u64,
                snapshot_id,
                "failed to reconstruct historical trie state from trie logs"
            );
            return None;
        }

        Some((requested_id, txn))
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("unused for now")
    }
}

impl BonsaiDB {
    fn replay_trie_logs_into_transaction(
        &self,
        txn: &mut BonsaiTransaction,
        snapshot_id: u64,
        requested_id: u64,
    ) -> Result<(), TrieError> {
        if snapshot_id == requested_id {
            return Ok(());
        }

        if snapshot_id > requested_id {
            for block_id in ((requested_id + 1)..=snapshot_id).rev() {
                let changes = self.load_trie_log_changes(BasicId::new(block_id))?;
                for ((key_kind, key), change) in changes {
                    txn.apply_change(key_kind, &key, change.old_value.as_deref())?;
                }
            }
            return Ok(());
        }

        for block_id in (snapshot_id + 1)..=requested_id {
            let changes = self.load_trie_log_changes(BasicId::new(block_id))?;
            for ((key_kind, key), change) in changes {
                txn.apply_change(key_kind, &key, change.new_value.as_deref())?;
            }
        }

        Ok(())
    }

    fn load_trie_log_changes(&self, block_id: BasicId) -> Result<BTreeMap<(u8, ByteVec), TrieLogChange>, TrieError> {
        let prefix = block_id.to_bytes();
        let trie_log_entries = self.get_by_prefix(&DatabaseKey::TrieLog(&prefix))?;
        let mut changes = BTreeMap::new();

        for (encoded_key, value) in trie_log_entries {
            let key_len = encoded_key.len();
            if key_len < prefix.len() + 3 {
                return Err(TrieError::InvalidTrieLogFormat(format!(
                    "key length {key_len} is shorter than prefix+metadata length {}",
                    prefix.len() + 3
                )));
            }
            if encoded_key[prefix.len()] != TRIE_LOG_KEY_SEPARATOR {
                return Err(TrieError::InvalidTrieLogFormat(format!(
                    "expected separator byte {TRIE_LOG_KEY_SEPARATOR:#04x} after prefix, got {:#04x}",
                    encoded_key[prefix.len()]
                )));
            }

            let change_type = encoded_key[key_len - 1];
            let key_kind = encoded_key[key_len - 2];
            let key_bytes = ByteVec::from(&encoded_key[prefix.len() + 1..key_len - 2]);
            let change = changes.entry((key_kind, key_bytes)).or_insert_with(TrieLogChange::default);

            match change_type {
                TRIE_LOG_NEW_VALUE => change.new_value = Some(value),
                TRIE_LOG_OLD_VALUE => change.old_value = Some(value),
                _ => {
                    return Err(TrieError::InvalidTrieLogFormat(format!("unsupported change type byte {change_type}")));
                }
            }
        }

        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitvec::view::AsBits;
    use starknet_types_core::felt::Felt;

    fn create_test_storage(config: crate::rocksdb::RocksDBConfig) -> (tempfile::TempDir, RocksDBStorage) {
        let temp_dir = tempfile::TempDir::with_prefix("bonsai-transaction-test").unwrap();
        let storage = RocksDBStorage::open(temp_dir.path(), config).unwrap();
        (temp_dir, storage)
    }

    #[test]
    fn contract_storage_transactional_state_uses_historical_snapshot_for_exact_block() {
        let (_temp_dir, storage) = create_test_storage(crate::rocksdb::RocksDBConfig {
            max_saved_trie_logs: Some(16),
            max_kept_snapshots: Some(16),
            snapshot_interval: 1,
            ..Default::default()
        });

        let contract = Felt::from_hex_unchecked("0x1234");
        let key = Felt::from_hex_unchecked("0x5678");
        let identifier = contract.to_bytes_be();
        let key_bits = key.to_bytes_be();

        let mut trie = storage.contract_storage_trie();
        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::ONE).unwrap();
        trie.commit(BasicId::new(0)).unwrap();
        storage.snapshots.set_new_head(0);
        let root_at_0 = trie.root_hash(&identifier).unwrap();

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::TWO).unwrap();
        trie.commit(BasicId::new(1)).unwrap();
        storage.snapshots.set_new_head(1);
        let root_at_1 = trie.root_hash(&identifier).unwrap();

        assert_ne!(root_at_0, root_at_1);

        let historical = storage
            .contract_storage_trie()
            .get_transactional_state(BasicId::new(0), storage.contract_storage_trie().get_config())
            .unwrap()
            .unwrap();

        assert_eq!(historical.root_hash(&identifier).unwrap(), root_at_0);
        assert_eq!(historical.get(&identifier, &key_bits.as_bits()[5..]).unwrap(), Some(Felt::ONE));
    }

    #[test]
    fn contract_storage_transactional_state_reconstructs_between_snapshots() {
        let (_temp_dir, storage) = create_test_storage(crate::rocksdb::RocksDBConfig {
            max_saved_trie_logs: Some(16),
            max_kept_snapshots: Some(16),
            snapshot_interval: 4,
            ..Default::default()
        });

        let contract = Felt::from_hex_unchecked("0x4321");
        let key = Felt::from_hex_unchecked("0x8765");
        let identifier = contract.to_bytes_be();
        let key_bits = key.to_bytes_be();

        let mut trie = storage.contract_storage_trie();

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::ONE).unwrap();
        trie.commit(BasicId::new(0)).unwrap();
        storage.snapshots.set_new_head(0);

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::TWO).unwrap();
        trie.commit(BasicId::new(1)).unwrap();
        storage.snapshots.set_new_head(1);
        let root_at_1 = trie.root_hash(&identifier).unwrap();

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::THREE).unwrap();
        trie.commit(BasicId::new(2)).unwrap();
        storage.snapshots.set_new_head(2);

        let historical = storage
            .contract_storage_trie()
            .get_transactional_state(BasicId::new(1), storage.contract_storage_trie().get_config())
            .unwrap()
            .unwrap();

        assert_eq!(historical.root_hash(&identifier).unwrap(), root_at_1);
        assert_eq!(historical.get(&identifier, &key_bits.as_bits()[5..]).unwrap(), Some(Felt::TWO));
    }

    #[test]
    fn contract_storage_transactional_state_reconstructs_multiple_reverse_steps() {
        let (_temp_dir, storage) = create_test_storage(crate::rocksdb::RocksDBConfig {
            max_saved_trie_logs: Some(16),
            max_kept_snapshots: Some(16),
            snapshot_interval: 5,
            ..Default::default()
        });

        let contract = Felt::from_hex_unchecked("0xaaaa");
        let key = Felt::from_hex_unchecked("0xbbbb");
        let identifier = contract.to_bytes_be();
        let key_bits = key.to_bytes_be();

        let mut trie = storage.contract_storage_trie();

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::ONE).unwrap();
        trie.commit(BasicId::new(0)).unwrap();
        storage.snapshots.set_new_head(0);

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::TWO).unwrap();
        trie.commit(BasicId::new(1)).unwrap();
        storage.snapshots.set_new_head(1);
        let root_at_1 = trie.root_hash(&identifier).unwrap();

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::THREE).unwrap();
        trie.commit(BasicId::new(2)).unwrap();
        storage.snapshots.set_new_head(2);

        trie.insert(&identifier, &key_bits.as_bits()[5..], &Felt::from(4_u8)).unwrap();
        trie.commit(BasicId::new(3)).unwrap();
        storage.snapshots.set_new_head(3);

        let historical = storage
            .contract_storage_trie()
            .get_transactional_state(BasicId::new(1), storage.contract_storage_trie().get_config())
            .unwrap()
            .unwrap();

        assert_eq!(historical.root_hash(&identifier).unwrap(), root_at_1);
        assert_eq!(historical.get(&identifier, &key_bits.as_bits()[5..]).unwrap(), Some(Felt::TWO));
    }
}
