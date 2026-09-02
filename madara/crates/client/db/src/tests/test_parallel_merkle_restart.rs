#![cfg(test)]

use crate::{
    preconfirmed::PreconfirmedBlock,
    rocksdb::{global_trie::in_memory::InMemoryRootComputation, RocksDBConfig},
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
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
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

fn write_parallel_block_parts(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    state_diffs_since_floor: &[StateDiff],
    include_overlay: bool,
) -> InMemoryRootComputation {
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: block_n, ..Default::default() }))
        .expect("creating preconfirmed block should succeed");

    let (base_block_n, snapshot) = backend
        .db
        .get_latest_durable_snapshot_floor(block_n.checked_sub(1))
        .expect("a durable checkpoint or empty-base snapshot should exist");
    let cumulative_state_diff =
        crate::rocksdb::global_trie::in_memory::squash_state_diffs(state_diffs_since_floor.iter());
    let computed = backend
        .db
        .compute_root_from_selected_snapshot(
            base_block_n,
            snapshot,
            block_n,
            &cumulative_state_diff,
            backend.chain_config().latest_protocol_version,
            include_overlay,
            false,
        )
        .expect("precomputing parallel Merkle root should succeed");

    backend
        .write_access()
        .write_preconfirmed_with_precomputed_root(
            false,
            block_n,
            state_diffs_since_floor.last().expect("current block state diff").clone(),
            computed.state_root,
            computed.timings.clone(),
        )
        .expect("writing block parts with a precomputed root should succeed");

    computed
}

fn assert_boundary_crash_recovered(backend: &Arc<MadaraBackend>, expected_confirmed_root: Felt) {
    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, Some(1));
    assert_eq!(head.external_preconfirmed_tip, Some(2));
    assert_eq!(head.internal_preconfirmed_tip, Some(2));
    assert_eq!(
        backend.db.get_state_root_hash().expect("reading recovered trie root should succeed"),
        expected_confirmed_root
    );
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(1));
    assert!(backend.has_parallel_merkle_checkpoint(1).expect("checkpoint 1"));
    assert!(!backend.has_parallel_merkle_checkpoint(2).expect("checkpoint 2"));
    assert_eq!(backend.db.get_latest_applied_trie_update().expect("latest trie update"), Some(1));
    assert!(backend.db.get_block_info(2).expect("reading rolled-back block parts").is_none());
    assert!(
        backend.db.get_preconfirmed_block_data(2).expect("reading preserved preconfirmed recovery log").is_some(),
        "preconfirmed block #2 must remain available for startup re-execution"
    );
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
        .compute_root_from_latest_snapshot(1, &diff1, backend.chain_config().latest_protocol_version, false)
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

#[test]
fn startup_cleans_preconfirmed_rows_left_behind_after_confirmed_head_advance() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    {
        let backend = open_backend(temp_dir.path());
        backend
            .write_access()
            .add_full_block_with_classes(&block_with_state_diff(0, synthetic_state_diff(0)), &[], false)
            .expect("closing block 0 should succeed");

        // This is the durable state left by a crash after the head projection was
        // advanced but before confirmed-path preconfirmed GC committed.
        backend
            .db
            .write_preconfirmed_header(&PreconfirmedHeader { block_number: 0, ..Default::default() })
            .expect("writing stale preconfirmed header should succeed");
        assert!(backend.db.get_preconfirmed_block_data(0).expect("reading stale preconfirmed row").is_some());
        backend.flush().expect("flushing stale preconfirmed fixture should succeed");
    }

    let reopened = open_backend(temp_dir.path());
    assert_eq!(reopened.chain_head_state().confirmed_tip, Some(0));
    assert_eq!(reopened.chain_head_state().external_preconfirmed_tip, None);
    assert_eq!(reopened.chain_head_state().internal_preconfirmed_tip, None);
    assert!(reopened.db.get_preconfirmed_block_data(0).expect("reading cleaned preconfirmed row").is_none());
}

#[test]
fn startup_rolls_boundary_trie_back_to_confirmed_head_and_preserves_preconfirmed_recovery_log() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let expected_confirmed_root = {
        let backend = open_backend(temp_dir.path());
        let diff0 = synthetic_state_diff(0);
        let diff1 = synthetic_state_diff(1);
        let diff2 = synthetic_state_diff(2);

        backend
            .write_access()
            .add_full_block_with_classes(&block_with_state_diff(0, diff0), &[], false)
            .expect("closing block 0 should succeed");
        backend.write_parallel_merkle_checkpoint(0).expect("checkpointing block 0 should succeed");
        backend.db.on_new_confirmed_head(0).expect("pinning block 0 snapshot should succeed");

        let block1 = write_parallel_block_parts(&backend, 1, std::slice::from_ref(&diff1), false);
        backend.write_access().new_confirmed_block(1).expect("confirming block 1 should succeed");

        let block2 = write_parallel_block_parts(&backend, 2, &[diff1, diff2], true);
        backend
            .db
            .flush_overlay_and_checkpoint(2, 3, Some(0), block2.overlay.as_ref().expect("boundary overlay"))
            .expect("persisting the block 2 boundary should succeed");

        assert_eq!(backend.chain_head_state().confirmed_tip, Some(1));
        assert_eq!(backend.db.get_state_root_hash().expect("boundary root"), block2.state_root);
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(2));
        backend.flush().expect("flushing boundary crash fixture should succeed");
        block1.state_root
    };

    {
        let reopened = open_backend(temp_dir.path());
        assert_boundary_crash_recovered(&reopened, expected_confirmed_root);
        reopened.flush().expect("flushing recovered database should succeed");
    }

    let reopened_again = open_backend(temp_dir.path());
    assert_boundary_crash_recovered(&reopened_again, expected_confirmed_root);
}

#[test]
fn startup_rolls_first_boundary_back_to_empty_base_before_replaying_confirmed_blocks() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let expected_confirmed_root = {
        let backend = open_backend(temp_dir.path());
        let diff0 = synthetic_state_diff(0);
        let diff1 = synthetic_state_diff(1);
        let diff2 = synthetic_state_diff(2);

        let block0 = write_parallel_block_parts(&backend, 0, std::slice::from_ref(&diff0), false);
        backend.write_access().new_confirmed_block(0).expect("confirming block 0 should succeed");

        let block1 = write_parallel_block_parts(&backend, 1, &[diff0.clone(), diff1.clone()], false);
        backend.write_access().new_confirmed_block(1).expect("confirming block 1 should succeed");

        assert_eq!(
            backend.db.get_parallel_merkle_checkpoint_floor(1).expect("checkpoint floor before first boundary"),
            None
        );
        assert_ne!(block0.state_root, Felt::ZERO);
        assert_eq!(backend.db.get_state_root_hash().expect("empty persisted trie"), Felt::ZERO);

        let block2 = write_parallel_block_parts(&backend, 2, &[diff0, diff1, diff2], true);
        backend
            .db
            .flush_overlay_and_checkpoint(2, 3, None, block2.overlay.as_ref().expect("first boundary overlay"))
            .expect("persisting the first boundary should succeed");

        assert_eq!(backend.chain_head_state().confirmed_tip, Some(1));
        assert_eq!(backend.db.get_state_root_hash().expect("first boundary root"), block2.state_root);
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(2));
        backend.flush().expect("flushing first-boundary crash fixture should succeed");
        block1.state_root
    };

    let reopened = open_backend(temp_dir.path());
    assert_boundary_crash_recovered(&reopened, expected_confirmed_root);
}
