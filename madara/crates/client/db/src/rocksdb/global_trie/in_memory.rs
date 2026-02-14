use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::SnapshotRef;
use crate::rocksdb::trie::{
    BasicId, TrieError, BONSAI_CLASS_FLAT_COLUMN, BONSAI_CLASS_LOG_COLUMN, BONSAI_CLASS_TRIE_COLUMN,
    BONSAI_CONTRACT_FLAT_COLUMN, BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
    BONSAI_CONTRACT_STORAGE_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_TRIE_COLUMN, BONSAI_CONTRACT_TRIE_COLUMN,
};
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};
use crate::{prelude::*, rocksdb::global_trie::ClassTrieTimings, rocksdb::global_trie::ContractTrieTimings};
use bitvec::{order::Msb0, vec::BitVec, view::AsBits};
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorageConfig, ByteVec, DatabaseKey};
use dashmap::DashMap;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use rayon::prelude::*;
use rocksdb::{Direction, IteratorMode};
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

const OVERLAY_TRIE_COLUMN_ID: u8 = 0;
const OVERLAY_FLAT_COLUMN_ID: u8 = 1;
const OVERLAY_TRIE_LOG_COLUMN_ID: u8 = 2;

pub(crate) type OverlayKey = (u8, ByteVec);
pub(crate) type OverlayMap = Arc<DashMap<OverlayKey, Option<ByteVec>>>;
type InMemoryTrie<H> = bonsai_trie::BonsaiStorage<BasicId, InMemoryBonsaiDb, H>;

#[derive(Clone, Debug)]
pub(crate) struct InMemoryColumnMapping {
    flat: Column,
    trie: Column,
    log: Column,
}

impl InMemoryColumnMapping {
    pub(crate) fn contract() -> Self {
        Self { flat: BONSAI_CONTRACT_FLAT_COLUMN, trie: BONSAI_CONTRACT_TRIE_COLUMN, log: BONSAI_CONTRACT_LOG_COLUMN }
    }

    pub(crate) fn contract_storage() -> Self {
        Self {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        }
    }

    pub(crate) fn class() -> Self {
        Self { flat: BONSAI_CLASS_FLAT_COLUMN, trie: BONSAI_CLASS_TRIE_COLUMN, log: BONSAI_CLASS_LOG_COLUMN }
    }

    fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }

    fn map_from_column_id(&self, column_id: u8) -> Option<&Column> {
        match column_id {
            OVERLAY_TRIE_COLUMN_ID => Some(&self.trie),
            OVERLAY_FLAT_COLUMN_ID => Some(&self.flat),
            OVERLAY_TRIE_LOG_COLUMN_ID => Some(&self.log),
            _ => None,
        }
    }
}

fn to_changed_key(key: &DatabaseKey) -> OverlayKey {
    (
        match key {
            DatabaseKey::Trie(_) => OVERLAY_TRIE_COLUMN_ID,
            DatabaseKey::Flat(_) => OVERLAY_FLAT_COLUMN_ID,
            DatabaseKey::TrieLog(_) => OVERLAY_TRIE_LOG_COLUMN_ID,
        },
        key.as_slice().into(),
    )
}

#[derive(Clone)]
pub(crate) struct InMemoryBonsaiDb {
    snapshot: SnapshotRef,
    changed: OverlayMap,
    column_mapping: InMemoryColumnMapping,
}

impl fmt::Debug for InMemoryBonsaiDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InMemoryBonsaiDb {{ changed_len: {} }}", self.changed.len())
    }
}

impl InMemoryBonsaiDb {
    pub(crate) fn with_mapping(
        snapshot: SnapshotRef,
        column_mapping: InMemoryColumnMapping,
        changed: OverlayMap,
    ) -> Self {
        Self { snapshot, changed, column_mapping }
    }

    pub(crate) fn contract(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract(), Arc::clone(&changed)), changed)
    }

    pub(crate) fn contract_storage(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract_storage(), Arc::clone(&changed)), changed)
    }

    pub(crate) fn class(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::class(), Arc::clone(&changed)), changed)
    }

    #[cfg(test)]
    fn test_with_mapping(snapshot: SnapshotRef, column_mapping: InMemoryColumnMapping) -> Self {
        Self::with_mapping(snapshot, column_mapping, Arc::new(DashMap::new()))
    }

    fn get_from_snapshot(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn changed_value(&self, key: &DatabaseKey) -> Option<Option<ByteVec>> {
        self.changed.get(&to_changed_key(key)).map(|v| v.value().clone())
    }
}

impl BonsaiDatabase for InMemoryBonsaiDb {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        if let Some(value) = self.changed_value(key) {
            return Ok(value);
        }
        self.get_from_snapshot(key)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        // We only need this for trie-log pruning in commit paths. We read from live DB here,
        // and rely on overlay-first entries to override current values for matching keys.
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(Vec::new());
        };
        let handle = self.snapshot.db.get_column(column.clone());

        let mut out: Vec<(ByteVec, ByteVec)> = self
            .snapshot
            .db
            .db
            .iterator_cf(&handle, IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward))
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix_bytes.as_slice()) {
                        Some((key.to_vec().into(), value.to_vec().into()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        for entry in self.changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if *column_id != prefix_col || !key.starts_with(prefix_bytes.as_slice()) {
                continue;
            }

            match value {
                Some(v) => {
                    if let Some((_, existing)) =
                        out.iter_mut().find(|(existing_key, _)| existing_key.as_slice() == key.as_slice())
                    {
                        *existing = v.clone();
                    } else {
                        out.push((key.clone(), v.clone()));
                    }
                }
                None => out.retain(|(existing_key, _)| existing_key.as_slice() != key.as_slice()),
            }
        }

        Ok(out)
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        if let Some(value) = self.changed_value(key) {
            return Ok(value.is_some());
        }
        Ok(self.get_from_snapshot(key)?.is_some())
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.get(key)?;
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(previous)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.get(key)?;
        self.changed.insert(to_changed_key(key), None);
        Ok(previous)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(());
        };
        let handle = self.snapshot.db.get_column(column.clone());

        for (key, _) in self
            .snapshot
            .db
            .db
            .iterator_cf(&handle, IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward))
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix_bytes.as_slice()) {
                        Some((key, value))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        {
            self.changed.insert((prefix_col, key.to_vec().into()), None);
        }

        for key in self
            .changed
            .iter()
            .map(|entry| entry.key().clone())
            .filter(|(column_id, key)| *column_id == prefix_col && key.starts_with(prefix_bytes.as_slice()))
            .collect::<Vec<_>>()
        {
            self.changed.insert(key, None);
        }

        Ok(())
    }

    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        // Intentionally a no-op: all writes stay in overlay until explicit flush.
        Ok(())
    }
}

impl BonsaiPersistentDatabase<BasicId> for InMemoryBonsaiDb {
    type Transaction<'a>
        = Self
    where
        Self: 'a;
    type DatabaseError = TrieError;

    fn snapshot(&mut self, _id: BasicId) {}

    fn transaction(&self, _id: BasicId) -> Option<(BasicId, Self::Transaction<'_>)> {
        None
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("merge is not supported for in-memory overlay db")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrieLogMode {
    #[default]
    Off,
    Checkpoint,
}

#[derive(Debug, Clone)]
pub struct BonsaiOverlay {
    pub contract_changed: OverlayMap,
    pub contract_storage_changed: OverlayMap,
    pub class_changed: OverlayMap,
}

impl BonsaiOverlay {
    fn apply_changed_map_to_batch(
        backend: &RocksDBStorage,
        mapping: &InMemoryColumnMapping,
        changed: &OverlayMap,
        trie_log_mode: TrieLogMode,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        for entry in changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if trie_log_mode == TrieLogMode::Off && *column_id == OVERLAY_TRIE_LOG_COLUMN_ID {
                continue;
            }

            let column = mapping
                .map_from_column_id(*column_id)
                .with_context(|| format!("unknown in-memory overlay column_id={column_id}"))?;
            let handle = backend.inner.get_column(column.clone());
            match value {
                Some(value) => batch.put_cf(&handle, key.as_slice(), value.as_slice()),
                None => batch.delete_cf(&handle, key.as_slice()),
            }
        }
        Ok(())
    }

    pub fn flush_to_db(&self, backend: &RocksDBStorage, trie_log_mode: TrieLogMode) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract(),
            &self.contract_changed,
            trie_log_mode,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract_storage(),
            &self.contract_storage_changed,
            trie_log_mode,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::class(),
            &self.class_changed,
            trie_log_mode,
            &mut batch,
        )?;

        backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct InMemoryRootComputation {
    pub block_n: u64,
    pub state_root: Felt,
    pub timings: crate::rocksdb::global_trie::MerklizationTimings,
    pub overlay: Option<BonsaiOverlay>,
}

#[derive(Debug, Default)]
struct StateDiffAccumulator {
    storage_diffs: HashMap<Felt, HashMap<Felt, Felt>>,
    contract_class_updates: HashMap<Felt, (Felt, bool)>, // (class_hash, from_deploy)
    class_hash_updates: HashMap<Felt, (Felt, bool)>,     // (compiled_class_hash, migrated)
    old_declared_classes: HashSet<Felt>,
    nonces: HashMap<Felt, Felt>,
}

impl StateDiffAccumulator {
    fn apply_state_diff(&mut self, state_diff: &StateDiff) {
        for ContractStorageDiffItem { address, storage_entries } in &state_diff.storage_diffs {
            let storage = self.storage_diffs.entry(*address).or_default();
            for StorageEntry { key, value } in storage_entries {
                storage.insert(*key, *value);
            }
        }

        for DeployedContractItem { address, class_hash } in &state_diff.deployed_contracts {
            self.contract_class_updates.insert(*address, (*class_hash, true));
        }
        for ReplacedClassItem { contract_address, class_hash } in &state_diff.replaced_classes {
            self.contract_class_updates.insert(*contract_address, (*class_hash, false));
        }

        for DeclaredClassItem { class_hash, compiled_class_hash } in &state_diff.declared_classes {
            self.class_hash_updates.insert(*class_hash, (*compiled_class_hash, false));
        }
        for MigratedClassItem { class_hash, compiled_class_hash } in &state_diff.migrated_compiled_classes {
            self.class_hash_updates.insert(*class_hash, (*compiled_class_hash, true));
        }

        self.old_declared_classes.extend(state_diff.old_declared_contracts.iter().copied());
        for NonceUpdate { contract_address, nonce } in &state_diff.nonces {
            self.nonces.insert(*contract_address, *nonce);
        }
    }

    fn to_state_diff(&self) -> StateDiff {
        let mut storage_diffs: Vec<_> = self
            .storage_diffs
            .iter()
            .map(|(address, entries)| {
                let mut storage_entries: Vec<_> =
                    entries.iter().map(|(key, value)| StorageEntry { key: *key, value: *value }).collect();
                storage_entries.sort_by_key(|entry| entry.key);
                ContractStorageDiffItem { address: *address, storage_entries }
            })
            .collect();
        storage_diffs.sort_by_key(|entry| entry.address);

        let mut deployed_contracts = Vec::new();
        let mut replaced_classes = Vec::new();
        for (address, (class_hash, from_deploy)) in self.contract_class_updates.iter() {
            if *from_deploy {
                deployed_contracts.push(DeployedContractItem { address: *address, class_hash: *class_hash });
            } else {
                replaced_classes.push(ReplacedClassItem { contract_address: *address, class_hash: *class_hash });
            }
        }
        deployed_contracts.sort_by_key(|item| item.address);
        replaced_classes.sort_by_key(|item| item.contract_address);

        let mut declared_classes = Vec::new();
        let mut migrated_compiled_classes = Vec::new();
        for (class_hash, (compiled_class_hash, migrated)) in self.class_hash_updates.iter() {
            if *migrated {
                migrated_compiled_classes
                    .push(MigratedClassItem { class_hash: *class_hash, compiled_class_hash: *compiled_class_hash });
            } else {
                declared_classes
                    .push(DeclaredClassItem { class_hash: *class_hash, compiled_class_hash: *compiled_class_hash });
            }
        }
        declared_classes.sort_by_key(|item| item.class_hash);
        migrated_compiled_classes.sort_by_key(|item| item.class_hash);

        let mut nonces: Vec<_> = self
            .nonces
            .iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address: *contract_address, nonce: *nonce })
            .collect();
        nonces.sort_by_key(|item| item.contract_address);

        let mut old_declared_contracts: Vec<_> = self.old_declared_classes.iter().copied().collect();
        old_declared_contracts.sort();

        StateDiff {
            storage_diffs,
            old_declared_contracts,
            declared_classes,
            deployed_contracts,
            replaced_classes,
            nonces,
            migrated_compiled_classes,
        }
    }
}

pub fn squash_state_diffs<'a>(state_diffs: impl IntoIterator<Item = &'a StateDiff>) -> StateDiff {
    let mut accumulator = StateDiffAccumulator::default();
    for state_diff in state_diffs {
        accumulator.apply_state_diff(state_diff);
    }
    accumulator.to_state_diff()
}

pub fn cumulative_squashed_state_diffs<'a>(state_diffs: impl IntoIterator<Item = &'a StateDiff>) -> Vec<StateDiff> {
    let mut accumulator = StateDiffAccumulator::default();
    let mut out = Vec::new();
    for state_diff in state_diffs {
        accumulator.apply_state_diff(state_diff);
        out.push(accumulator.to_state_diff());
    }
    out
}

#[derive(Debug, Default)]
struct ContractLeaf {
    class_hash: Option<Felt>,
    storage_root: Option<Felt>,
    nonce: Option<Felt>,
}

fn bonsai_storage_config_for_mode(backend: &RocksDBStorage, trie_log_mode: TrieLogMode) -> BonsaiStorageConfig {
    BonsaiStorageConfig {
        max_saved_trie_logs: match trie_log_mode {
            TrieLogMode::Off => Some(0),
            TrieLogMode::Checkpoint => backend.inner.config.max_saved_trie_logs,
        },
        max_saved_snapshots: Some(0),
        snapshot_interval: backend.inner.config.snapshot_interval,
    }
}

fn contract_state_leaf_hash(
    backend: &RocksDBStorage,
    contract_address: &Felt,
    contract_leaf: &ContractLeaf,
    block_number: u64,
) -> Result<Felt> {
    let nonce = contract_leaf
        .nonce
        .unwrap_or(backend.inner.get_contract_nonce_at(block_number, contract_address)?.unwrap_or(Felt::ZERO));

    let class_hash = if let Some(class_hash) = contract_leaf.class_hash {
        class_hash
    } else {
        backend.inner.get_contract_class_hash_at(block_number, contract_address)?.unwrap_or(Felt::ZERO)
    };
    let storage_root =
        contract_leaf.storage_root.context("Storage root needs to be set before contract leaf hashing")?;

    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}

fn in_memory_contract_trie_root(
    backend: &RocksDBStorage,
    contract_storage_trie: &mut InMemoryTrie<Pedersen>,
    contract_trie: &mut InMemoryTrie<Pedersen>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ContractTrieTimings)> {
    let mut timings = ContractTrieTimings::default();
    let mut contract_leafs: HashMap<Felt, ContractLeaf> = HashMap::new();

    for ContractStorageDiffItem { address, storage_entries } in &state_diff.storage_diffs {
        for StorageEntry { key, value } in storage_entries {
            let bytes = key.to_bytes_be();
            let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            contract_storage_trie
                .insert(&address.to_bytes_be(), &bitvec, value)
                .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
        }
        contract_leafs.insert(*address, ContractLeaf::default());
    }

    let storage_commit_start = Instant::now();
    contract_storage_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.storage_commit = storage_commit_start.elapsed();

    for NonceUpdate { contract_address, nonce } in &state_diff.nonces {
        contract_leafs.entry(*contract_address).or_default().nonce = Some(*nonce);
    }
    for DeployedContractItem { address, class_hash } in &state_diff.deployed_contracts {
        contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
    }
    for ReplacedClassItem { contract_address, class_hash } in &state_diff.replaced_classes {
        contract_leafs.entry(*contract_address).or_default().class_hash = Some(*class_hash);
    }

    let leaf_hashes: Vec<_> = contract_leafs
        .into_par_iter()
        .map(|(contract_address, mut leaf)| {
            let storage_root = contract_storage_trie
                .root_hash(&contract_address.to_bytes_be())
                .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
            leaf.storage_root = Some(storage_root);
            let leaf_hash = contract_state_leaf_hash(backend, &contract_address, &leaf, block_n)?;
            let bytes = contract_address.to_bytes_be();
            let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            anyhow::Ok((bitvec, leaf_hash))
        })
        .collect::<Result<_>>()?;

    for (key, value) in leaf_hashes {
        contract_trie
            .insert(super::bonsai_identifier::CONTRACT, &key, &value)
            .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    }

    let trie_commit_start = Instant::now();
    contract_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.trie_commit = trie_commit_start.elapsed();

    let root = contract_trie
        .root_hash(super::bonsai_identifier::CONTRACT)
        .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    Ok((root, timings))
}

const CONTRACT_CLASS_HASH_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

fn compute_class_leaf_hash(compiled_class_hash: &Felt) -> Felt {
    Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)
}

fn in_memory_class_trie_root(
    class_trie: &mut InMemoryTrie<Poseidon>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ClassTrieTimings)> {
    let mut timings = ClassTrieTimings::default();

    let declared_updates: Vec<_> = state_diff
        .declared_classes
        .par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            (*class_hash, compute_class_leaf_hash(compiled_class_hash))
        })
        .collect();

    let migrated_updates: Vec<_> = state_diff
        .migrated_compiled_classes
        .par_iter()
        .map(|MigratedClassItem { class_hash, compiled_class_hash }| {
            (*class_hash, compute_class_leaf_hash(compiled_class_hash))
        })
        .collect();

    for (key, value) in declared_updates.into_iter().chain(migrated_updates) {
        let bytes = key.to_bytes_be();
        let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie
            .insert(super::bonsai_identifier::CLASS, &bitvec, &value)
            .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    }

    let trie_commit_start = Instant::now();
    class_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.trie_commit = trie_commit_start.elapsed();
    let root =
        class_trie.root_hash(super::bonsai_identifier::CLASS).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    Ok((root, timings))
}

pub fn compute_root_from_snapshot(
    backend: &RocksDBStorage,
    snapshot: SnapshotRef,
    block_n: u64,
    state_diff: &StateDiff,
    include_overlay: bool,
    trie_log_mode: TrieLogMode,
) -> Result<InMemoryRootComputation> {
    let config = bonsai_storage_config_for_mode(backend, trie_log_mode);
    let (contract_db, contract_changed) = InMemoryBonsaiDb::contract(Arc::clone(&snapshot));
    let (contract_storage_db, contract_storage_changed) = InMemoryBonsaiDb::contract_storage(Arc::clone(&snapshot));
    let (class_db, class_changed) = InMemoryBonsaiDb::class(snapshot);

    let mut contract_trie: InMemoryTrie<Pedersen> = bonsai_trie::BonsaiStorage::new(contract_db, config.clone(), 251);
    let mut contract_storage_trie: InMemoryTrie<Pedersen> =
        bonsai_trie::BonsaiStorage::new(contract_storage_db, config.clone(), 251);
    let mut class_trie: InMemoryTrie<Poseidon> = bonsai_trie::BonsaiStorage::new(class_db, config, 251);

    let block_start = Instant::now();
    let ((contract_result, contract_duration), (class_result, class_duration)) = rayon::join(
        || {
            let start = Instant::now();
            let result = in_memory_contract_trie_root(
                backend,
                &mut contract_storage_trie,
                &mut contract_trie,
                state_diff,
                block_n,
            );
            (result, start.elapsed())
        },
        || {
            let start = Instant::now();
            let result = in_memory_class_trie_root(&mut class_trie, state_diff, block_n);
            (result, start.elapsed())
        },
    );

    let (contract_root, contract_trie_timings) = contract_result?;
    let (class_root, class_trie_timings) = class_result?;
    let state_root = super::calculate_state_root(contract_root, class_root);

    let timings = crate::rocksdb::global_trie::MerklizationTimings {
        total: block_start.elapsed(),
        contract_trie_root: contract_duration,
        class_trie_root: class_duration,
        contract_trie: contract_trie_timings,
        class_trie: class_trie_timings,
    };

    let overlay =
        include_overlay.then_some(BonsaiOverlay { contract_changed, contract_storage_changed, class_changed });
    Ok(InMemoryRootComputation { block_n, state_root, timings, overlay })
}

pub fn compute_roots_in_parallel_from_snapshot(
    backend: &RocksDBStorage,
    snapshot: SnapshotRef,
    start_block_n: u64,
    state_diffs: &[StateDiff],
    boundary_block_n: Option<u64>,
    trie_log_mode: TrieLogMode,
) -> Result<Vec<InMemoryRootComputation>> {
    let cumulative = cumulative_squashed_state_diffs(state_diffs.iter());
    let mut roots: Vec<_> = cumulative
        .into_par_iter()
        .enumerate()
        .map(|(index, state_diff)| {
            let block_n = start_block_n + u64::try_from(index).expect("index fits into u64");
            compute_root_from_snapshot(
                backend,
                Arc::clone(&snapshot),
                block_n,
                &state_diff,
                boundary_block_n == Some(block_n),
                trie_log_mode,
            )
        })
        .collect::<Result<_>>()?;

    roots.sort_by_key(|result| result.block_n);
    Ok(roots)
}

pub fn flush_overlay_and_checkpoint(
    backend: &RocksDBStorage,
    block_n: u64,
    overlay: &BonsaiOverlay,
    trie_log_mode: TrieLogMode,
) -> Result<()> {
    let mut batch = WriteBatchWithTransaction::default();
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract(),
        &overlay.contract_changed,
        trie_log_mode,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract_storage(),
        &overlay.contract_storage_changed,
        trie_log_mode,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::class(),
        &overlay.class_changed,
        trie_log_mode,
        &mut batch,
    )?;
    backend.inner.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
    backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::rocksdb_snapshot::SnapshotWithDBArc;
    use crate::rocksdb::RocksDBStorage;
    use crate::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
        ReplacedClassItem, StateDiff, StorageEntry,
    };
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    fn setup_backend() -> Arc<MadaraBackend> {
        MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()))
    }

    fn setup_snapshot_db() -> Arc<MadaraBackend> {
        setup_backend()
    }

    fn write_snapshot_value(backend: &RocksDBStorage, column: Column, key: &[u8], value: &[u8]) {
        let handle = backend.inner.get_column(column);
        backend.inner.db.put_cf(&handle, key, value).expect("write snapshot value");
    }

    fn fresh_snapshot(backend: &RocksDBStorage) -> SnapshotRef {
        Arc::new(SnapshotWithDBArc::new(Arc::clone(&backend.inner)))
    }

    fn count_column_entries(backend: &RocksDBStorage, column: Column) -> usize {
        let handle = backend.inner.get_column(column);
        backend.inner.db.iterator_cf(&handle, IteratorMode::Start).map_while(|item| item.ok()).count()
    }

    fn synthetic_state_diff(index: u64) -> StateDiff {
        let contract_address = Felt::from(10_000 + index);
        let class_hash = Felt::from(20_000 + index);
        let compiled_class_hash = Felt::from(30_000 + index);

        StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: contract_address,
                storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(40_000 + index) }],
            }],
            old_declared_contracts: vec![],
            declared_classes: vec![DeclaredClassItem { class_hash, compiled_class_hash }],
            deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
            replaced_classes: vec![],
            nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(index + 1) }],
            migrated_compiled_classes: vec![],
        }
    }

    fn sequential_roots(backend: &Arc<MadaraBackend>, diffs: &[StateDiff]) -> Vec<Felt> {
        let mut roots = Vec::new();
        for (index, diff) in diffs.iter().enumerate() {
            let block_n = u64::try_from(index).expect("index fits into u64");
            let (root, _timings) = backend
                .write_access()
                .apply_to_global_trie(block_n, [diff])
                .expect("sequential apply_to_global_trie should succeed");
            roots.push(root);
        }
        roots
    }

    #[test]
    fn in_memory_bonsai_overlay_hit_beats_snapshot() {
        let backend = setup_snapshot_db();
        let key = b"overlay-hit-key";
        write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"snapshot-value");
        let snapshot = fresh_snapshot(&backend.db);
        let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

        let got_from_snapshot = db.get(&DatabaseKey::Flat(key)).expect("read from snapshot");
        assert_eq!(got_from_snapshot, Some(ByteVec::from(&b"snapshot-value"[..])));

        db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay");
        let got = db.get(&DatabaseKey::Flat(key)).expect("read overlay");
        assert_eq!(got, Some(ByteVec::from(&b"overlay-value"[..])));
    }

    #[test]
    fn in_memory_bonsai_tombstone_hides_snapshot_value() {
        let backend = setup_snapshot_db();
        let key = b"tombstone-key";
        write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"snapshot-value");
        let snapshot = fresh_snapshot(&backend.db);
        let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

        assert!(db.contains(&DatabaseKey::Flat(key)).expect("contains before delete"));
        db.remove(&DatabaseKey::Flat(key), None).expect("remove key");

        assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("read tombstoned key"), None);
        assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains after delete"));
    }

    #[test]
    fn in_memory_bonsai_insert_remove_contains_are_consistent() {
        let backend = setup_snapshot_db();
        let snapshot = fresh_snapshot(&backend.db);
        let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
        let key = b"in-memory-key";

        assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains before insert"));
        db.insert(&DatabaseKey::Flat(key), b"value-1", None).expect("insert");
        assert!(db.contains(&DatabaseKey::Flat(key)).expect("contains after insert"));
        assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after insert"), Some(ByteVec::from(&b"value-1"[..])));

        db.insert(&DatabaseKey::Flat(key), b"value-2", None).expect("overwrite");
        assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after overwrite"), Some(ByteVec::from(&b"value-2"[..])));

        db.remove(&DatabaseKey::Flat(key), None).expect("remove");
        assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after remove"), None);
        assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains after remove"));
    }

    #[test]
    fn in_memory_bonsai_write_batch_does_not_persist_to_rocksdb() {
        let backend = setup_snapshot_db();
        let snapshot = fresh_snapshot(&backend.db);
        let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
        let key = b"not-persisted-key";
        let handle = backend.db.inner.get_column(BONSAI_CONTRACT_FLAT_COLUMN);

        db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay value");
        db.write_batch(WriteBatchWithTransaction::default()).expect("write batch no-op");

        let persisted = backend.db.inner.db.get_cf(&handle, key).expect("read rocksdb");
        assert_eq!(persisted, None, "overlay writes must not persist before explicit flush");
    }

    #[test]
    fn in_memory_single_block_root_matches_sequential_apply() {
        let backend_seq = setup_backend();
        let backend_mem = setup_backend();
        let diff = synthetic_state_diff(0);

        let expected_root = sequential_roots(&backend_seq, std::slice::from_ref(&diff))[0];
        let snapshot = fresh_snapshot(&backend_mem.db);
        let computed =
            compute_root_from_snapshot(&backend_mem.db, snapshot, 0, &diff, false, TrieLogMode::Off).expect("compute");

        assert_eq!(computed.state_root, expected_root);
        assert!(computed.overlay.is_none(), "overlay should be absent when include_overlay=false");
    }

    #[test]
    fn cumulative_squash_keeps_root_relevant_fields() {
        let contract_address = Felt::from(777_u64);
        let diff_a = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: contract_address,
                storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(11_u64) }],
            }],
            old_declared_contracts: vec![],
            declared_classes: vec![DeclaredClassItem {
                class_hash: Felt::from(100_u64),
                compiled_class_hash: Felt::from(200_u64),
            }],
            deployed_contracts: vec![DeployedContractItem {
                address: contract_address,
                class_hash: Felt::from(100_u64),
            }],
            replaced_classes: vec![],
            nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(1_u64) }],
            migrated_compiled_classes: vec![],
        };
        let diff_b = StateDiff {
            storage_diffs: vec![ContractStorageDiffItem {
                address: contract_address,
                storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(22_u64) }],
            }],
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![ReplacedClassItem { contract_address, class_hash: Felt::from(101_u64) }],
            nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(2_u64) }],
            migrated_compiled_classes: vec![MigratedClassItem {
                class_hash: Felt::from(101_u64),
                compiled_class_hash: Felt::from(201_u64),
            }],
        };

        let squashed = squash_state_diffs([&diff_a, &diff_b]);
        assert_eq!(squashed.storage_diffs.len(), 1);
        assert_eq!(
            squashed.storage_diffs[0].storage_entries,
            vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(22_u64) }]
        );
        assert_eq!(squashed.nonces, vec![NonceUpdate { contract_address, nonce: Felt::from(2_u64) }]);
        assert_eq!(
            squashed.replaced_classes,
            vec![ReplacedClassItem { contract_address, class_hash: Felt::from(101_u64) }]
        );
        assert_eq!(
            squashed.migrated_compiled_classes,
            vec![MigratedClassItem { class_hash: Felt::from(101_u64), compiled_class_hash: Felt::from(201_u64) }]
        );
    }

    #[test]
    fn in_memory_parallel_roots_match_sequential_per_block() {
        let backend_seq = setup_backend();
        let backend_mem = setup_backend();
        let diffs: Vec<_> = (0_u64..5).map(synthetic_state_diff).collect();

        let expected_roots = sequential_roots(&backend_seq, &diffs);
        let snapshot = fresh_snapshot(&backend_mem.db);
        let results = compute_roots_in_parallel_from_snapshot(
            &backend_mem.db,
            snapshot,
            0,
            &diffs,
            Some(2),
            TrieLogMode::Checkpoint,
        )
        .expect("parallel roots");

        let got_roots: Vec<_> = results.iter().map(|result| result.state_root).collect();
        assert_eq!(got_roots, expected_roots);
        assert_eq!(results.iter().filter(|result| result.overlay.is_some()).count(), 1);
        assert_eq!(results.iter().find(|result| result.overlay.is_some()).map(|result| result.block_n), Some(2));
    }

    #[test]
    fn boundary_flush_updates_persisted_root_and_checkpoint() {
        let backend = setup_backend();
        let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();
        let snapshot = fresh_snapshot(&backend.db);

        let results =
            compute_roots_in_parallel_from_snapshot(&backend.db, snapshot, 0, &diffs, Some(2), TrieLogMode::Checkpoint)
                .expect("parallel roots");
        let boundary = results.last().expect("boundary result");
        let overlay = boundary.overlay.as_ref().expect("boundary overlay");

        flush_overlay_and_checkpoint(&backend.db, boundary.block_n, overlay, TrieLogMode::Checkpoint)
            .expect("flush and checkpoint");

        let persisted_root = crate::rocksdb::global_trie::get_state_root(&backend.db).expect("read persisted root");
        assert_eq!(persisted_root, boundary.state_root);
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(2));
        assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint marker"));
    }

    #[test]
    fn trie_log_mode_controls_log_column_flush_behavior() {
        let backend_off = setup_backend();
        let backend_checkpoint = setup_backend();
        let diff = synthetic_state_diff(0);

        let off_snapshot = fresh_snapshot(&backend_off.db);
        let off_result = compute_root_from_snapshot(&backend_off.db, off_snapshot, 0, &diff, true, TrieLogMode::Off)
            .expect("off mode compute");
        flush_overlay_and_checkpoint(
            &backend_off.db,
            0,
            off_result.overlay.as_ref().expect("off overlay"),
            TrieLogMode::Off,
        )
        .expect("off mode flush");

        let checkpoint_snapshot = fresh_snapshot(&backend_checkpoint.db);
        let checkpoint_result = compute_root_from_snapshot(
            &backend_checkpoint.db,
            checkpoint_snapshot,
            0,
            &diff,
            true,
            TrieLogMode::Checkpoint,
        )
        .expect("checkpoint mode compute");
        flush_overlay_and_checkpoint(
            &backend_checkpoint.db,
            0,
            checkpoint_result.overlay.as_ref().expect("checkpoint overlay"),
            TrieLogMode::Checkpoint,
        )
        .expect("checkpoint mode flush");

        let off_log_entries = count_column_entries(&backend_off.db, BONSAI_CONTRACT_LOG_COLUMN)
            + count_column_entries(&backend_off.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
            + count_column_entries(&backend_off.db, BONSAI_CLASS_LOG_COLUMN);
        let checkpoint_log_entries = count_column_entries(&backend_checkpoint.db, BONSAI_CONTRACT_LOG_COLUMN)
            + count_column_entries(&backend_checkpoint.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
            + count_column_entries(&backend_checkpoint.db, BONSAI_CLASS_LOG_COLUMN);

        assert_eq!(off_log_entries, 0, "off mode should not persist trie logs");
        assert!(checkpoint_log_entries > 0, "checkpoint mode should persist trie logs");
    }

    #[test]
    fn checkpoint_metadata_must_be_monotonic() {
        let backend = setup_backend();
        backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");
        backend.write_parallel_merkle_checkpoint(8).expect("checkpoint 8");

        let err = backend.write_parallel_merkle_checkpoint(7).expect_err("checkpoint regression should be rejected");
        let message = format!("{err:#}");
        assert!(message.contains("must be monotonic"), "unexpected error: {message}");
    }
}
