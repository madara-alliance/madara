#![cfg(test)]

use crate::{
    preconfirmed::PreconfirmedBlock,
    rocksdb::{global_trie::in_memory::TrieLogMode, RocksDBConfig},
    storage::{MadaraStorageRead, MadaraStorageWrite},
    MadaraBackend, MadaraBackendConfig,
};
use mc_class_exec::config::NativeConfig;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry};
use starknet_types_core::felt::Felt;
use std::{path::Path, sync::Arc};

fn open_backend(path: &Path) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        Arc::new(NativeConfig::default()),
    )
    .expect("opening RocksDB backend should succeed")
}

fn synthetic_state_diff(index: u64) -> StateDiff {
    let contract_address = Felt::from(10_000 + index);
    let class_hash = Felt::from(20_000 + index);
    StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(40_000 + index) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(index + 1) }],
        migrated_compiled_classes: vec![],
    }
}

fn block_with_state_diff(block_number: u64, state_diff: StateDiff) -> FullBlockWithoutCommitments {
    FullBlockWithoutCommitments {
        header: PreconfirmedHeader { block_number, ..Default::default() },
        state_diff,
        transactions: vec![],
        events: vec![],
    }
}

fn create_non_durable_confirmed_head(backend: &Arc<MadaraBackend>) -> Felt {
    let diff0 = synthetic_state_diff(0);
    let diff1 = synthetic_state_diff(1);

    backend
        .write_access()
        .add_full_block_with_classes(&block_with_state_diff(0, diff0), &[], false)
        .expect("closing block 0 should succeed");
    backend.write_parallel_merkle_checkpoint(0).expect("checkpointing block 0 should succeed");
    backend.db.on_new_confirmed_head(0).expect("pinning block 0 snapshot should succeed");

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating block 1 preconfirmed state should succeed");

    let computed = backend
        .db
        .compute_root_from_latest_snapshot(1, &diff1, false, TrieLogMode::Checkpoint)
        .expect("precomputing block 1 root should succeed");
    let expected_root = computed.state_root;

    backend
        .write_access()
        .write_preconfirmed_with_precomputed_root(false, 1, diff1, computed.state_root, computed.timings)
        .expect("writing precomputed block 1 should succeed");
    backend.write_access().new_confirmed_block(1).expect("confirming block 1 should succeed");

    let block_info =
        backend.db.get_block_info(1).expect("reading block 1 info should succeed").expect("block 1 exists");
    assert_eq!(block_info.header.global_state_root, expected_root);
    assert_ne!(
        backend.db.get_state_root_hash().expect("reading persisted trie root should succeed"),
        expected_root,
        "test setup must leave the trie behind the confirmed head"
    );

    expected_root
}

#[test]
fn shutdown_reconcile_makes_non_boundary_confirmed_head_durable() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let backend = open_backend(temp_dir.path());
    let expected_root = create_non_durable_confirmed_head(&backend);

    backend.reconcile_confirmed_parallel_merkle_state("test_shutdown").expect("shutdown reconciliation should succeed");

    assert_eq!(backend.db.get_state_root_hash().expect("reading reconciled trie root should succeed"), expected_root);
    assert!(backend.has_parallel_merkle_checkpoint(1).expect("reading checkpoint state should succeed"));
    assert_eq!(backend.db.get_latest_durable_snapshot_floor(Some(1)).map(|(block_n, _)| block_n), Some(Some(1)));
}

#[test]
fn startup_reconciles_non_boundary_confirmed_head_on_reopen() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let expected_root = {
        let backend = open_backend(temp_dir.path());
        let expected_root = create_non_durable_confirmed_head(&backend);
        backend.flush().expect("flushing non-durable head fixture should succeed");
        expected_root
    };

    let reopened = open_backend(temp_dir.path());
    let block_info =
        reopened.db.get_block_info(1).expect("reading reopened block info should succeed").expect("block 1 exists");

    assert_eq!(reopened.chain_head_state().confirmed_tip, Some(1));
    assert_eq!(block_info.header.global_state_root, expected_root);
    assert_eq!(reopened.db.get_state_root_hash().expect("reading reopened trie root should succeed"), expected_root);
    assert!(reopened.has_parallel_merkle_checkpoint(1).expect("reading reopened checkpoint should succeed"));
    assert_eq!(reopened.db.get_latest_durable_snapshot_floor(Some(1)).map(|(block_n, _)| block_n), Some(Some(1)));
}
