#![cfg(test)]

use crate::{
    preconfirmed::PreconfirmedBlock,
    rocksdb::{
        global_trie::in_memory::{BoundaryFlushOutcome, InMemoryRootComputation},
        trie::BasicId,
        RocksDBConfig,
    },
    storage::{MadaraStorageRead, MadaraStorageWrite, StorageHeadProjection},
    MadaraBackend, MadaraBackendConfig,
};
use mc_class_exec::config::NativeConfig;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry};
use starknet_types_core::felt::Felt;
use std::{path::Path, sync::Arc};

/// Opens the shared RocksDB fixture without selecting a service-specific recovery policy.
/// Tests use this to model the interval between database open and service startup.
fn open_backend_without_trie_reconciliation(path: &Path) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        RocksDBConfig::default(),
        Arc::new(NativeConfig::default()),
    )
    .expect("opening RocksDB backend should succeed")
}

/// Opens a block-production fixture and applies the parallel-Merkle startup policy.
/// This mirrors the recovery performed by `BlockProductionTask` when that mode is enabled.
fn open_backend(path: &Path) -> Arc<MadaraBackend> {
    let backend = open_backend_without_trie_reconciliation(path);
    backend
        .reconcile_confirmed_parallel_merkle_state("test_block_production_startup")
        .expect("parallel-Merkle startup reconciliation should succeed");
    backend
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

/// Publishes one prepared boundary overlay against its declared durable base.
fn publish_boundary(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    base_block_n: Option<u64>,
    computed: &InMemoryRootComputation,
) -> BoundaryFlushOutcome {
    backend
        .db
        .flush_overlay_and_checkpoint(
            block_n,
            3,
            base_block_n,
            computed.overlay.as_ref().expect("boundary computation should include an overlay"),
        )
        .expect("publishing boundary overlay should succeed")
}

/// Confirms the prepared block and advances snapshot bookkeeping exactly once.
fn confirm_parallel_block(backend: &Arc<MadaraBackend>, block_n: u64) {
    backend.write_access().new_confirmed_block(block_n).expect("confirming prepared block should succeed");
}

/// Builds and publishes the first cumulative boundary at block 2.
fn confirm_initial_boundary(backend: &Arc<MadaraBackend>, diffs: &[StateDiff]) {
    for block_n in 0_u64..=2 {
        let computed = write_parallel_block_parts(backend, block_n, &diffs[..=block_n as usize], block_n == 2);
        if block_n == 2 {
            assert_eq!(publish_boundary(backend, block_n, None, &computed), BoundaryFlushOutcome::Persisted);
        }
        confirm_parallel_block(backend, block_n);
    }
}

/// Computes block 8 from checkpoint 2 before publishing checkpoint 5.
fn compute_stale_block_8_before_checkpoint_5(
    backend: &Arc<MadaraBackend>,
    diffs: &[StateDiff],
) -> InMemoryRootComputation {
    for block_n in 3_u64..=4 {
        write_parallel_block_parts(backend, block_n, &diffs[3..=block_n as usize], false);
        confirm_parallel_block(backend, block_n);
    }

    let (base_block_n, snapshot) =
        backend.db.get_latest_durable_snapshot_floor(Some(2)).expect("checkpoint 2 should have a durable snapshot");
    assert_eq!(base_block_n, Some(2));
    let cumulative_diff = crate::rocksdb::global_trie::in_memory::squash_state_diffs(diffs[3..=8].iter());
    let stale_block_8 = backend
        .db
        .compute_root_from_selected_snapshot(
            base_block_n,
            snapshot,
            8,
            &cumulative_diff,
            backend.chain_config().latest_protocol_version,
            true,
            false,
        )
        .expect("computing block 8 from checkpoint 2 should succeed");

    let block_5 = write_parallel_block_parts(backend, 5, &diffs[3..=5], true);
    assert_eq!(publish_boundary(backend, 5, Some(2), &block_5), BoundaryFlushOutcome::Persisted);
    confirm_parallel_block(backend, 5);
    stale_block_8
}

/// Skips the stale block-8 overlay, then publishes a six-block catch-up at block 11.
fn confirm_stale_skip_and_catch_up(
    backend: &Arc<MadaraBackend>,
    diffs: &[StateDiff],
    stale_block_8: &InMemoryRootComputation,
) -> (Felt, Felt) {
    for block_n in 6_u64..=7 {
        write_parallel_block_parts(backend, block_n, &diffs[6..=block_n as usize], false);
        confirm_parallel_block(backend, block_n);
    }

    let block_8 = write_parallel_block_parts(backend, 8, &diffs[6..=8], true);
    assert_eq!(block_8.state_root, stale_block_8.state_root);
    assert_eq!(
        publish_boundary(backend, 8, Some(2), stale_block_8),
        BoundaryFlushOutcome::StaleBaseSkipped { latest_checkpoint: 5 }
    );
    confirm_parallel_block(backend, 8);

    for block_n in 9_u64..=10 {
        write_parallel_block_parts(backend, block_n, &diffs[6..=block_n as usize], false);
        confirm_parallel_block(backend, block_n);
    }

    let block_11 = write_parallel_block_parts(backend, 11, &diffs[6..=11], true);
    assert_eq!(publish_boundary(backend, 11, Some(5), &block_11), BoundaryFlushOutcome::Persisted);
    confirm_parallel_block(backend, 11);

    let target = backend.db.get_block_info(8).expect("reading block 8").expect("block 8 should exist");
    (target.block_hash, target.header.global_state_root)
}

/// Reverts twice to block 8 and verifies that the second call is a no-op.
fn assert_revert_is_idempotent(backend: &Arc<MadaraBackend>, target_hash: Felt, target_root: Felt) {
    let first_revert = backend.revert_to(&target_hash).expect("reverting to block 8 should succeed");
    assert_eq!(first_revert, (8, target_hash));
    assert_eq!(backend.db.get_state_root_hash().expect("reading reverted root"), target_root);
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(8));
    assert!(!backend.has_parallel_merkle_checkpoint(11).expect("checkpoint 11 should be removed"));

    let repeated_revert = backend.revert_to(&target_hash).expect("repeating the same revert should be a no-op");
    assert_eq!(repeated_revert, (8, target_hash));
    assert_eq!(backend.db.get_state_root_hash().expect("reading root after repeated revert"), target_root);
}

/// Verifies that the reverted target remains canonical after reopening RocksDB.
fn assert_revert_survives_reopen(path: &Path, target_hash: Felt, target_root: Felt) {
    let reopened = open_backend(path);
    assert_eq!(reopened.chain_head_state().confirmed_tip, Some(8));
    assert_eq!(reopened.db.get_state_root_hash().expect("reading reopened root"), target_root);
    assert_eq!(reopened.get_parallel_merkle_latest_checkpoint().expect("reopened checkpoint"), Some(8));
    assert_eq!(
        reopened
            .db
            .get_block_info(8)
            .expect("reading reopened block 8")
            .expect("reopened block 8 should exist")
            .block_hash,
        target_hash
    );
    assert!(reopened.db.get_block_info(9).expect("reading reverted block 9").is_none());
}

/// Simulates a crash after different tries reached different reorg revisions.
fn leave_interrupted_revert_state(backend: &Arc<MadaraBackend>) {
    let mut contract_trie = backend.db.contract_trie_for_revert();
    contract_trie
        .revert_to(BasicId::new(2), BasicId::new(11))
        .expect("partially reverting contract trie should succeed");
    contract_trie.commit(BasicId::new(2)).expect("committing partially reverted contract trie should succeed");

    let mut contract_storage_trie = backend.db.contract_storage_trie_for_revert();
    contract_storage_trie
        .revert_to(BasicId::new(5), BasicId::new(11))
        .expect("partially reverting contract-storage trie should succeed");
    contract_storage_trie
        .commit(BasicId::new(5))
        .expect("committing partially reverted contract-storage trie should succeed");
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
fn stale_boundary_skip_then_cumulative_commit_reverts_idempotently_after_reopen() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let (target_hash, target_root) = {
        let backend = open_backend(temp_dir.path());
        let diffs: Vec<_> = (0_u64..=11).map(synthetic_state_diff).collect();

        confirm_initial_boundary(&backend, &diffs);
        let stale_block_8 = compute_stale_block_8_before_checkpoint_5(&backend, &diffs);
        let (target_hash, target_root) = confirm_stale_skip_and_catch_up(&backend, &diffs, &stale_block_8);
        assert_revert_is_idempotent(&backend, target_hash, target_root);
        backend.flush().expect("flushing reverted database should succeed");
        (target_hash, target_root)
    };

    assert_revert_survives_reopen(temp_dir.path(), target_hash, target_root);
}

#[test]
fn startup_recovers_confirmed_tip_after_interrupted_revert() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let (confirmed_hash, confirmed_root, revert_hash, revert_root) = {
        let backend = open_backend(temp_dir.path());
        let diffs: Vec<_> = (0_u64..=11).map(synthetic_state_diff).collect();

        confirm_initial_boundary(&backend, &diffs);
        let stale_block_8 = compute_stale_block_8_before_checkpoint_5(&backend, &diffs);
        confirm_stale_skip_and_catch_up(&backend, &diffs, &stale_block_8);

        let confirmed = backend.db.get_block_info(11).expect("reading confirmed tip").expect("confirmed tip exists");
        let revert_target = backend.db.get_block_info(2).expect("reading revert target").expect("revert target exists");
        leave_interrupted_revert_state(&backend);
        backend.flush().expect("flushing interrupted revert fixture should succeed");

        (
            confirmed.block_hash,
            confirmed.header.global_state_root,
            revert_target.block_hash,
            revert_target.header.global_state_root,
        )
    };

    let recovered = open_backend(temp_dir.path());
    assert_eq!(recovered.chain_head_state().confirmed_tip, Some(11));
    assert_eq!(recovered.db.get_state_root_hash().expect("reading recovered root"), confirmed_root);
    assert_eq!(
        recovered.db.get_block_info(11).expect("reading recovered tip").expect("recovered tip exists").block_hash,
        confirmed_hash
    );
    assert_eq!(recovered.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(11));
    // Ordered recovery now recreates verified checkpoints as it replays each block.
    assert!(recovered.has_parallel_merkle_checkpoint(5).expect("replayed checkpoint 5"));
    assert!(recovered.has_parallel_merkle_checkpoint(8).expect("replayed checkpoint 8"));

    let first_revert = recovered.revert_to(&revert_hash).expect("retrying interrupted revert should succeed");
    assert_eq!(first_revert, (2, revert_hash));
    assert_eq!(recovered.db.get_state_root_hash().expect("reading reverted root"), revert_root);

    let repeated_revert = recovered.revert_to(&revert_hash).expect("repeating completed revert should succeed");
    assert_eq!(repeated_revert, (2, revert_hash));
    assert_eq!(recovered.db.get_state_root_hash().expect("reading repeated revert root"), revert_root);
}

#[test]
fn startup_finishes_reverse_suffix_cleanup_after_reorg_head_commit() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let (target_hash, target_root) = {
        let backend = open_backend(temp_dir.path());
        let diffs: Vec<_> = (0_u64..=11).map(synthetic_state_diff).collect();

        confirm_initial_boundary(&backend, &diffs);
        let stale_block_8 = compute_stale_block_8_before_checkpoint_5(&backend, &diffs);
        confirm_stale_skip_and_catch_up(&backend, &diffs, &stale_block_8);

        let target = backend.db.get_block_info(2).expect("reading target block").expect("target block exists");
        let mut contract_trie = backend.db.contract_trie_for_revert();
        contract_trie
            .revert_to(BasicId::new(2), BasicId::new(11))
            .expect("reverting contract trie to committed target");
        contract_trie.commit(BasicId::new(2)).expect("committing reverted contract trie");
        let mut contract_storage_trie = backend.db.contract_storage_trie_for_revert();
        contract_storage_trie
            .revert_to(BasicId::new(2), BasicId::new(11))
            .expect("reverting storage trie to committed target");
        contract_storage_trie.commit(BasicId::new(2)).expect("committing reverted storage trie");

        // Simulate a crash after the target-head commit and after only the newest
        // part of the reverse-order suffix cleanup completed. Blocks 3..=8 remain.
        backend.db.replace_head_projection(&StorageHeadProjection::Confirmed(2)).expect("publishing target head");
        backend.db.write_latest_applied_trie_update(&Some(2)).expect("publishing trie cursor");
        backend.db.remove_all_blocks_starting_from(9).expect("removing newest suffix before simulated crash");
        assert!(backend.db.get_block_info(8).expect("reading remaining suffix").is_some());
        assert!(backend.db.get_block_info(9).expect("reading removed suffix").is_none());
        backend.flush().expect("flushing simulated post-commit crash state");

        (target.block_hash, target.header.global_state_root)
    };

    let reopened = open_backend(temp_dir.path());
    assert_eq!(reopened.chain_head_state().confirmed_tip, Some(2));
    assert_eq!(reopened.db.get_state_root_hash().expect("reading recovered root"), target_root);
    assert_eq!(reopened.db.get_block_info(2).expect("reading target").expect("target exists").block_hash, target_hash);
    for block_n in 3_u64..=11 {
        assert!(
            reopened.db.get_block_info(block_n).expect("reading cleaned suffix").is_none(),
            "startup must finish cleanup for block #{block_n}"
        );
    }

    let repeated = reopened.revert_to(&target_hash).expect("repeating completed revert");
    assert_eq!(repeated, (2, target_hash));
    assert_eq!(reopened.db.get_state_root_hash().expect("reading repeated root"), target_root);
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

#[test]
fn full_node_startup_preserves_independent_sync_pipeline_progress() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let (confirmed_root, runahead_root) = {
        let backend = open_backend_without_trie_reconciliation(temp_dir.path());
        backend
            .write_access()
            .add_full_block_with_classes(&block_with_state_diff(0, synthetic_state_diff(0)), &[], false)
            .expect("importing confirmed block 0 should succeed");
        let confirmed_root = backend
            .db
            .get_block_info(0)
            .expect("reading confirmed block info should succeed")
            .expect("confirmed block 0 should exist")
            .header
            .global_state_root;

        let (runahead_root, _) = backend
            .write_access()
            .apply_to_global_trie(1, [&synthetic_state_diff(1)], backend.chain_config().latest_protocol_version)
            .expect("applying an unsealed sync batch should succeed");
        backend.write_latest_applied_trie_update(&Some(1)).expect("writing trie runahead cursor should succeed");
        backend.write_snap_sync_latest_block(&Some(1)).expect("writing SnapSync runahead cursor should succeed");
        backend.flush().expect("flushing full-node runahead fixture should succeed");

        assert_ne!(runahead_root, confirmed_root, "fixture must leave trie state ahead of the confirmed head");
        (confirmed_root, runahead_root)
    };

    let reopened = open_backend_without_trie_reconciliation(temp_dir.path());
    assert_eq!(reopened.chain_head_state().confirmed_tip, Some(0));
    assert_eq!(reopened.db.get_state_root_hash().expect("reading runahead root should succeed"), runahead_root);

    assert_ne!(runahead_root, confirmed_root);
    assert_eq!(reopened.get_latest_applied_trie_update().expect("reading trie cursor"), Some(1));
    assert_eq!(reopened.get_snap_sync_latest_block().expect("reading SnapSync cursor"), Some(1));
}

#[test]
fn same_tip_revert_discards_persisted_runahead_across_restart() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let backend = open_backend(temp_dir.path());
    let block_hash = crate::test_utils::add_test_block(&backend, 0, vec![]);
    for block_number in 1..=3 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number, ..Default::default() }))
            .expect("persist preconfirmed suffix");
    }

    backend.revert_to(&block_hash).expect("discard suffix at current confirmed tip");
    assert_eq!(backend.latest_confirmed_block_n(), Some(0));
    assert!(!backend.has_preconfirmed_block());
    for block_number in 1..=3 {
        assert!(backend.db.get_preconfirmed_block_data(block_number).unwrap().is_none());
    }
    drop(backend);

    let reopened = open_backend(temp_dir.path());
    assert_eq!(reopened.latest_confirmed_block_n(), Some(0));
    assert!(!reopened.has_preconfirmed_block());
}

fn same_contract_storage_diff(entries: &[(u64, u64)], deploy: bool) -> StateDiff {
    let address = Felt::from(777_u64);
    StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address,
            storage_entries: entries
                .iter()
                .map(|&(key, value)| StorageEntry { key: Felt::from(key), value: Felt::from(value) })
                .collect(),
        }],
        deployed_contracts: if deploy {
            vec![DeployedContractItem { address, class_hash: Felt::from(888_u64) }]
        } else {
            vec![]
        },
        ..Default::default()
    }
}

fn import_serial_storage_block(backend: &Arc<MadaraBackend>, number: u64, diff: &StateDiff) -> Felt {
    backend
        .write_access()
        .add_full_block_with_classes(&block_with_state_diff(number, diff.clone()), &[], false)
        .expect("import serial block");
    backend.db.get_state_root_hash().unwrap()
}

#[rstest::rstest]
#[case::serial(false, 1)]
#[case::parallel_checkpoint(true, 0)]
#[case::parallel_floor_replay(true, 1)]
fn cached_storage_revert_then_sparse_update_matches_reference(
    #[case] parallel: bool,
    #[case] target: u64,
    #[values(false, true)] resume_parallel: bool,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path());
    let reference = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let diffs = [
        same_contract_storage_diff(&[(1, 10), (2, 20), (3, 30)], true),
        same_contract_storage_diff(&[(2, 200)], false),
        same_contract_storage_diff(&[(3, 300)], false),
        same_contract_storage_diff(&[(2, 222)], false),
    ];
    import_serial_storage_block(&backend, 0, &diffs[0]);
    if parallel {
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        backend.db.on_new_confirmed_head(0).unwrap();
    }
    for number in 1..=3 {
        if parallel {
            let computed = write_parallel_block_parts(&backend, number, &diffs[1..=number as usize], number == 3);
            if number == 3 {
                assert_eq!(publish_boundary(&backend, number, Some(0), &computed), BoundaryFlushOutcome::Persisted);
            }
            confirm_parallel_block(&backend, number);
        } else {
            import_serial_storage_block(&backend, number, &diffs[number as usize]);
        }
    }
    let target_info = backend.db.get_block_info(target).unwrap().unwrap();
    backend.revert_to(&target_info.block_hash).expect("revert target");
    assert_eq!(backend.db.get_state_root_hash().unwrap(), target_info.header.global_state_root);
    for number in 0..=target {
        import_serial_storage_block(&reference, number, &diffs[number as usize]);
    }
    let next = same_contract_storage_diff(&[(1, 100)], false);
    let expected = import_serial_storage_block(&reference, target + 1, &next);
    let actual = if resume_parallel {
        // A serial reorg may not create checkpoint metadata itself.
        backend.write_parallel_merkle_checkpoint(target).unwrap();
        backend.db.on_new_confirmed_head(target).unwrap();
        let computed = write_parallel_block_parts(&backend, target + 1, &[next], true);
        assert_eq!(publish_boundary(&backend, target + 1, Some(target), &computed), BoundaryFlushOutcome::Persisted);
        confirm_parallel_block(&backend, target + 1);
        backend.db.get_state_root_hash().unwrap()
    } else {
        import_serial_storage_block(&backend, target + 1, &next)
    };
    assert_eq!(actual, expected, "post-revert sparse update must preserve sibling storage");
}

#[rstest::rstest]
fn cached_storage_reconcile_rollback_then_sparse_update_matches_reference(#[values(false, true)] empty: bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path());
    let reference = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let base = same_contract_storage_diff(&[(1, 10), (2, 20)], true);
    if !empty {
        import_serial_storage_block(&backend, 0, &base);
        import_serial_storage_block(&reference, 0, &base);
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        backend.db.on_new_confirmed_head(0).unwrap();
    }
    let runahead = if empty { base } else { same_contract_storage_diff(&[(2, 200)], false) };
    backend
        .write_access()
        .apply_to_global_trie(u64::from(!empty), [&runahead], backend.chain_config().latest_protocol_version)
        .unwrap();
    backend.reconcile_confirmed_parallel_merkle_state("test_cache_rollback").unwrap();
    assert_eq!(backend.db.get_state_root_hash().unwrap(), reference.db.get_state_root_hash().unwrap());
    let next = same_contract_storage_diff(&[(1, 100)], empty);
    let expected = import_serial_storage_block(&reference, u64::from(!empty), &next);
    let actual = import_serial_storage_block(&backend, u64::from(!empty), &next);
    assert_eq!(actual, expected, "reconciliation must discard cached runahead storage before serial resume");
}

#[test]
fn cached_storage_parallel_reconcile_then_serial_resume_matches_reference() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path());
    let reference = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let base = same_contract_storage_diff(&[(1, 10), (2, 20), (3, 30)], true);
    let confirmed = same_contract_storage_diff(&[(2, 200)], false);
    let runahead = same_contract_storage_diff(&[(3, 300)], false);
    import_serial_storage_block(&backend, 0, &base);
    import_serial_storage_block(&reference, 0, &base);
    let confirmed_root = import_serial_storage_block(&reference, 1, &confirmed);
    backend.write_parallel_merkle_checkpoint(0).unwrap();
    backend.db.on_new_confirmed_head(0).unwrap();
    let block1 = write_parallel_block_parts(&backend, 1, std::slice::from_ref(&confirmed), false);
    assert_eq!(block1.state_root, confirmed_root);
    confirm_parallel_block(&backend, 1);
    let block2 = write_parallel_block_parts(&backend, 2, &[confirmed, runahead], true);
    assert_eq!(publish_boundary(&backend, 2, Some(0), &block2), BoundaryFlushOutcome::Persisted);
    // The boundary reached disk before block 2 was confirmed. Reuse this live backend,
    // as service shutdown/startup does, rather than masking the cache by reopening it.
    backend.reconcile_confirmed_parallel_merkle_state("test_cache_parallel_runahead").unwrap();
    assert_eq!(backend.db.get_state_root_hash().unwrap(), confirmed_root);
    let next = same_contract_storage_diff(&[(1, 100)], false);
    let expected = import_serial_storage_block(&reference, 2, &next);
    let actual = import_serial_storage_block(&backend, 2, &next);
    assert_eq!(actual, expected, "serial resume must use the reconciled parallel state");
}

#[test]
fn cached_storage_parallel_floor_revert_then_boundary_then_serial_matches_reference() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path());
    let reference = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let base = same_contract_storage_diff(&[(1, 10), (2, 20), (3, 30)], true);
    let keep = same_contract_storage_diff(&[(2, 200)], false);
    let discard = same_contract_storage_diff(&[(3, 300)], false);
    import_serial_storage_block(&backend, 0, &base);
    import_serial_storage_block(&reference, 0, &base);
    let target_root = import_serial_storage_block(&reference, 1, &keep);
    backend.write_parallel_merkle_checkpoint(0).unwrap();
    backend.db.on_new_confirmed_head(0).unwrap();
    write_parallel_block_parts(&backend, 1, std::slice::from_ref(&keep), false);
    confirm_parallel_block(&backend, 1);
    let boundary = write_parallel_block_parts(&backend, 2, &[keep, discard], true);
    assert_eq!(publish_boundary(&backend, 2, Some(0), &boundary), BoundaryFlushOutcome::Persisted);
    confirm_parallel_block(&backend, 2);
    let target_hash = backend.db.get_block_info(1).unwrap().unwrap().block_hash;
    backend.revert_to(&target_hash).unwrap();
    assert_eq!(backend.db.get_state_root_hash().unwrap(), target_root);
    // Floor replay used the serial writer, warming its cache at block 1.
    let next_parallel = same_contract_storage_diff(&[(2, 222)], false);
    let expected_parallel = import_serial_storage_block(&reference, 2, &next_parallel);
    let boundary = write_parallel_block_parts(&backend, 2, &[next_parallel], true);
    assert_eq!(boundary.state_root, expected_parallel);
    assert_eq!(publish_boundary(&backend, 2, Some(1), &boundary), BoundaryFlushOutcome::Persisted);
    confirm_parallel_block(&backend, 2);
    let next_serial = same_contract_storage_diff(&[(1, 100)], false);
    let expected = import_serial_storage_block(&reference, 3, &next_serial);
    let actual = import_serial_storage_block(&backend, 3, &next_serial);
    assert_eq!(actual, expected, "serial resume after reorg and parallel boundary must use the latest state");
}

fn assert_storage_proof_at(backend: &MadaraBackend, block: u64, expected_root: Felt, values: &[u64]) {
    use bitvec::{order::Msb0, view::AsBits};
    use starknet_types_core::hash::Pedersen;

    let trie = backend.db.contract_storage_trie();
    let mut historical = trie.get_transactional_state(BasicId::new(block), trie.get_config()).unwrap().unwrap();
    let address = Felt::from(777_u64).to_bytes_be();
    let root = historical.root_hash(&address).unwrap();
    assert_eq!(root, expected_root, "proof root must belong to requested block {block}");
    let keys: Vec<_> = (1..=values.len()).map(|key| Felt::from(key as u64).to_bytes_be()).collect();
    let proof = historical.get_multi_proof(&address, keys.iter().map(|key| &key.as_bits::<Msb0>()[5..])).unwrap();
    let proven = proof
        .verify_proof::<Pedersen>(root, keys.iter().map(|key| &key.as_bits::<Msb0>()[5..]), 251)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(proven, values.iter().copied().map(Felt::from).collect::<Vec<_>>());
}

#[rstest::rstest]
fn historical_storage_proof_preserves_overwritten_and_deleted_values(#[values(false, true)] parallel: bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path());
    let base = same_contract_storage_diff(&[(1, 10), (2, 20)], true);
    import_serial_storage_block(&backend, 0, &base);
    let address = Felt::from(777_u64).to_bytes_be();
    let root0 = backend.db.contract_storage_trie().root_hash(&address).unwrap();
    let diffs = [same_contract_storage_diff(&[(1, 0), (2, 200)], false), same_contract_storage_diff(&[(3, 30)], false)];
    if parallel {
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        backend.db.on_new_confirmed_head(0).unwrap();
    }
    for block in 1..=2 {
        if parallel {
            let computed = write_parallel_block_parts(&backend, block, &diffs[..block as usize], block == 2);
            if block == 2 {
                assert_eq!(publish_boundary(&backend, block, Some(0), &computed), BoundaryFlushOutcome::Persisted);
            }
            confirm_parallel_block(&backend, block);
        } else {
            import_serial_storage_block(&backend, block, &diffs[block as usize - 1]);
        }
    }
    let root2 = backend.db.contract_storage_trie().root_hash(&address).unwrap();
    assert_storage_proof_at(&backend, 2, root2, &[0, 200, 30, 0]);
    assert_storage_proof_at(&backend, 0, root0, &[10, 20, 0, 0]);
}

#[test]
fn storage_proof_after_delete_revert_and_reinsert_matches_values() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    let address = Felt::from(777_u64).to_bytes_be();
    import_serial_storage_block(&backend, 0, &same_contract_storage_diff(&[(1, 10), (2, 20)], true));
    let block0 = backend.db.get_block_info(0).unwrap().unwrap();
    let root0 = backend.db.contract_storage_trie().root_hash(&address).unwrap();
    import_serial_storage_block(&backend, 1, &same_contract_storage_diff(&[(1, 0), (2, 200)], false));
    let root1 = backend.db.contract_storage_trie().root_hash(&address).unwrap();
    assert_storage_proof_at(&backend, 1, root1, &[0, 200, 0]);
    backend.revert_to(&block0.block_hash).unwrap();
    assert_storage_proof_at(&backend, 0, root0, &[10, 20, 0]);
    import_serial_storage_block(&backend, 1, &same_contract_storage_diff(&[(1, 100), (3, 30)], false));
    let root1 = backend.db.contract_storage_trie().root_hash(&address).unwrap();
    assert_storage_proof_at(&backend, 1, root1, &[100, 20, 30, 0]);
}
