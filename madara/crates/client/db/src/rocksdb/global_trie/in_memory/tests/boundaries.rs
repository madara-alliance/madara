use super::*;

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
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash: Felt::from(100_u64) }],
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
        None,
        snapshot,
        0,
        &diffs,
        protocol_version(),
        Some(2),
    )
    .expect("parallel roots");

    let got_roots: Vec<_> = results.iter().map(|result| result.state_root).collect();
    assert_eq!(got_roots, expected_roots);
    assert_eq!(results.iter().filter(|result| result.overlay.is_some()).count(), 1);
    assert_eq!(results.iter().find(|result| result.overlay.is_some()).map(|result| result.block_n), Some(2));
}

#[test]
fn contract_leaf_fallback_reads_nonce_and_class_from_snapshot_state() {
    let contract_address = Felt::from(777_u64);
    let original_nonce = Felt::from(11_u64);
    let original_class_hash = Felt::from(22_u64);
    let mutated_nonce = Felt::from(33_u64);
    let mutated_class_hash = Felt::from(44_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash: original_class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: original_nonce }],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(55_u64) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let expected = compute_root_from_snapshot(
        &backend_expected.db,
        Some(0),
        fresh_snapshot(&backend_expected.db),
        1,
        &current_diff,
        protocol_version(),
        false,
    )
    .expect("expected compute");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let snapshot = fresh_snapshot(&backend_actual.db);

    let nonce_handle = backend_actual.db.inner.get_column(CONTRACT_NONCE_COLUMN);
    let class_handle = backend_actual.db.inner.get_column(CONTRACT_CLASS_HASH_COLUMN);
    backend_actual
        .db
        .inner
        .db
        .put_cf(
            &nonce_handle,
            make_contract_history_key(&contract_address, 1),
            crate::rocksdb::serialize_to_smallvec::<[u8; 64]>(&mutated_nonce).expect("serialize nonce"),
        )
        .expect("mutate live nonce history");
    backend_actual
        .db
        .inner
        .db
        .put_cf(
            &class_handle,
            make_contract_history_key(&contract_address, 1),
            crate::rocksdb::serialize_to_smallvec::<[u8; 64]>(&mutated_class_hash).expect("serialize class hash"),
        )
        .expect("mutate live class history");

    let actual =
        compute_root_from_snapshot(&backend_actual.db, Some(0), snapshot, 1, &current_diff, protocol_version(), false)
            .expect("actual compute");

    assert_eq!(actual.state_root, expected.state_root, "snapshot-scoped fallback should ignore live DB mutation");
}

#[test]
fn boundary_flush_updates_persisted_root_and_checkpoint() {
    let backend = setup_backend();
    let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();
    let snapshot = fresh_snapshot(&backend.db);

    let results =
        compute_roots_in_parallel_from_snapshot(&backend.db, None, snapshot, 0, &diffs, protocol_version(), Some(2))
            .expect("parallel roots");
    let boundary = results.last().expect("boundary result");
    let overlay = boundary.overlay.as_ref().expect("boundary overlay");

    flush_overlay_and_checkpoint(&backend.db, boundary.block_n, 3, None, overlay).expect("flush and checkpoint");

    let persisted_root =
        crate::rocksdb::global_trie::get_state_root(&backend.db, protocol_version()).expect("read persisted root");
    assert_eq!(persisted_root, boundary.state_root);
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(2));
    assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint marker"));
}

#[test]
fn stale_boundary_overlay_is_skipped_and_later_boundary_catches_up() {
    let backend = setup_backend();
    let all_diffs: Vec<_> = (0_u64..9).map(synthetic_state_diff).collect();

    let first_boundary = compute_boundary(&backend.db, None, fresh_snapshot(&backend.db), 2, &all_diffs[..3]);
    flush_overlay_and_checkpoint(
        &backend.db,
        2,
        3,
        None,
        first_boundary.overlay.as_ref().expect("first boundary overlay"),
    )
    .expect("flush first boundary");

    let shared_base = fresh_snapshot(&backend.db);
    let block5 = compute_boundary(&backend.db, Some(2), Arc::clone(&shared_base), 5, &all_diffs[3..6]);
    let block8_from_2 = compute_boundary(&backend.db, Some(2), shared_base, 8, &all_diffs[3..9]);

    let block5_outcome =
        flush_overlay_and_checkpoint(&backend.db, 5, 3, Some(2), block5.overlay.as_ref().expect("block 5 overlay"))
            .expect("flush block 5 boundary");
    assert_eq!(block5_outcome, BoundaryFlushOutcome::Persisted);

    let stale_outcome = flush_overlay_and_checkpoint(
        &backend.db,
        8,
        3,
        Some(2),
        block8_from_2.overlay.as_ref().expect("stale block 8 overlay"),
    )
    .expect("skip stale block 8 boundary");
    assert_eq!(stale_outcome, BoundaryFlushOutcome::StaleBaseSkipped { latest_checkpoint: 5 });
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("checkpoint after skip"), Some(5));

    let block8_from_5 = compute_boundary(&backend.db, Some(5), fresh_snapshot(&backend.db), 8, &all_diffs[6..9]);
    assert_eq!(block8_from_5.state_root, block8_from_2.state_root);
    flush_overlay_and_checkpoint(
        &backend.db,
        8,
        3,
        Some(5),
        block8_from_5.overlay.as_ref().expect("fresh block 8 overlay"),
    )
    .expect("flush block 8 boundary from current checkpoint");
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("checkpoint after catch-up"), Some(8));
}

#[test]
fn boundary_flush_always_persists_trie_logs() {
    let backend = setup_backend();
    let diff = synthetic_state_diff(0);

    let snapshot = fresh_snapshot(&backend.db);
    let result = compute_root_from_snapshot(&backend.db, None, snapshot, 0, &diff, protocol_version(), true)
        .expect("root compute");
    flush_overlay_and_checkpoint(&backend.db, 0, 1, None, result.overlay.as_ref().expect("boundary overlay"))
        .expect("boundary flush");

    let log_entries = count_column_entries(&backend.db, BONSAI_CONTRACT_LOG_COLUMN)
        + count_column_entries(&backend.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
        + count_column_entries(&backend.db, BONSAI_CLASS_LOG_COLUMN);

    assert!(log_entries > 0, "boundary recovery requires persisted trie logs");
}

#[test]
fn parallel_merkle_warns_but_allows_disabled_trie_log_retention() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let storage = RocksDBStorage::open(
        temp_dir.path(),
        crate::rocksdb::RocksDBConfig { max_saved_trie_logs: Some(0), ..Default::default() },
    )
    .expect("open storage");
    let diff = synthetic_state_diff(0);

    compute_root_from_snapshot(&storage, None, fresh_snapshot(&storage), 0, &diff, protocol_version(), true)
        .expect("parallel Merkle should remain available when trie logs are explicitly disabled");
}

#[test]
fn boundary_log_retention_counts_block_revisions() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let storage = RocksDBStorage::open(
        temp_dir.path(),
        crate::rocksdb::RocksDBConfig { max_saved_trie_logs: Some(4), ..Default::default() },
    )
    .expect("open storage");
    let boundary_interval = 3;
    let mut base_block_n = None;

    for boundary_block_n in [2_u64, 5, 8] {
        let start_block_n = boundary_block_n + 1 - boundary_interval;
        let diffs: Vec<_> = (start_block_n..=boundary_block_n).map(synthetic_state_diff).collect();
        let results = compute_roots_in_parallel_from_snapshot(
            &storage,
            base_block_n,
            fresh_snapshot(&storage),
            start_block_n,
            &diffs,
            protocol_version(),
            Some(boundary_block_n),
        )
        .expect("compute boundary roots");
        let overlay = results.last().and_then(|result| result.overlay.as_ref()).expect("boundary overlay");
        flush_overlay_and_checkpoint(&storage, boundary_block_n, boundary_interval, base_block_n, overlay)
            .expect("flush boundary overlay");
        base_block_n = Some(boundary_block_n);
    }

    let revision_entry_count = |revision| {
        [BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN, BONSAI_CLASS_LOG_COLUMN]
            .into_iter()
            .map(|column| count_revision_entries(&storage, column, revision))
            .sum::<usize>()
    };

    assert_eq!(revision_entry_count(2), 0, "revision outside the four-block window should be pruned");
    assert!(revision_entry_count(5) > 0, "revision inside the four-block window should remain");
    assert!(revision_entry_count(8) > 0, "latest boundary logs should remain");
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

#[test]
fn checkpoint_metadata_floor_and_cleanup_follow_revert_target() {
    let backend = setup_backend();
    backend.write_parallel_merkle_checkpoint(2).expect("checkpoint 2");
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");
    backend.write_parallel_merkle_checkpoint(8).expect("checkpoint 8");

    assert_eq!(
        backend.db.get_parallel_merkle_checkpoint_floor(7).expect("floor"),
        Some(5),
        "floor should pick greatest checkpoint <= target"
    );

    backend.db.remove_parallel_merkle_checkpoints_above(5).expect("remove checkpoints above target");

    assert_eq!(
        backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"),
        Some(5),
        "latest checkpoint pointer should be rewound to target floor"
    );
    assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint 2 must remain"));
    assert!(backend.has_parallel_merkle_checkpoint(5).expect("checkpoint 5 must remain"));
    assert!(!backend.has_parallel_merkle_checkpoint(8).expect("checkpoint 8 must be removed"));
}
