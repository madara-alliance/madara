use super::*;
use crate::rocksdb::column::Column;
use crate::rocksdb::rocksdb_snapshot::SnapshotWithDBArc;
use crate::rocksdb::state::{CONTRACT_CLASS_HASH_COLUMN, CONTRACT_NONCE_COLUMN};
use crate::rocksdb::trie::{
    BONSAI_CLASS_LOG_COLUMN, BONSAI_CONTRACT_FLAT_COLUMN, BONSAI_CONTRACT_LOG_COLUMN,
    BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
};
use crate::rocksdb::RocksDBStorage;
use crate::MadaraBackend;
use bonsai_trie::{BonsaiDatabase, ByteVec, DatabaseKey};
use dashmap::DashMap;
use mp_chain_config::{ChainConfig, StarknetVersion};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use rocksdb::IteratorMode;
use starknet_types_core::felt::Felt;
use std::fs;
use std::mem::size_of;
use std::sync::Arc;

use crate::rocksdb::snapshots::SnapshotRef;

fn setup_backend() -> Arc<MadaraBackend> {
    MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()))
}

fn setup_snapshot_db() -> Arc<MadaraBackend> {
    setup_backend()
}

fn protocol_version() -> StarknetVersion {
    ChainConfig::madara_test().latest_protocol_version
}

fn write_snapshot_value(backend: &RocksDBStorage, column: Column, key: &[u8], value: &[u8]) {
    let handle = backend.inner.get_column(column);
    backend.inner.db.put_cf(&handle, key, value).expect("write snapshot value");
}

fn fresh_snapshot(backend: &RocksDBStorage) -> SnapshotRef {
    Arc::new(SnapshotWithDBArc::new(Arc::clone(&backend.inner)))
}

fn load_state_diff_fixture(path: &str) -> StateDiff {
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|_| panic!("read fixture {path}")))
            .unwrap_or_else(|_| panic!("parse fixture json {path}"));
    let obj = value.as_object_mut().expect("state diff fixture must be an object");
    obj.entry("old_declared_contracts").or_insert_with(|| serde_json::json!([]));
    obj.entry("replaced_classes").or_insert_with(|| serde_json::json!([]));
    obj.entry("declared_classes").or_insert_with(|| serde_json::json!([]));
    obj.entry("migrated_compiled_classes").or_insert_with(|| serde_json::json!([]));
    serde_json::from_value(value).unwrap_or_else(|_| panic!("parse state diff fixture {path}"))
}

fn count_column_entries(backend: &RocksDBStorage, column: Column) -> usize {
    let handle = backend.inner.get_column(column);
    backend.inner.db.iterator_cf(&handle, IteratorMode::Start).map_while(|item| item.ok()).count()
}

fn count_revision_entries(backend: &RocksDBStorage, column: Column, revision: u64) -> usize {
    let handle = backend.inner.get_column(column);
    let prefix = revision.to_be_bytes();
    backend
        .inner
        .db
        .iterator_cf(&handle, IteratorMode::Start)
        .map(|item| item.expect("read trie-log entry"))
        .filter(|(key, _)| key.starts_with(&prefix))
        .count()
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

/// Computes one boundary overlay from a chosen durable snapshot and diff span.
fn compute_boundary(
    backend: &RocksDBStorage,
    base_block_n: Option<u64>,
    snapshot: SnapshotRef,
    block_n: u64,
    state_diffs: &[StateDiff],
) -> InMemoryRootComputation {
    let cumulative_diff = squash_state_diffs(state_diffs.iter());
    compute_root_from_snapshot(backend, base_block_n, snapshot, block_n, &cumulative_diff, protocol_version(), true)
        .expect("compute boundary root")
}

fn make_contract_history_key(contract_address: &Felt, block_n: u32) -> [u8; 32 + size_of::<u32>()] {
    let mut key = [0u8; 32 + size_of::<u32>()];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..].copy_from_slice(&(u32::MAX - block_n).to_be_bytes());
    key
}

fn sequential_roots(backend: &Arc<MadaraBackend>, diffs: &[StateDiff]) -> Vec<Felt> {
    let mut roots = Vec::new();
    for (index, diff) in diffs.iter().enumerate() {
        let block_n = u64::try_from(index).expect("index fits into u64");
        let (root, _timings) = backend
            .write_access()
            .apply_to_global_trie(block_n, [diff], protocol_version())
            .expect("sequential apply_to_global_trie should succeed");
        roots.push(root);
    }
    roots
}

mod boundaries;
mod overlay;
mod roots;
mod search;
