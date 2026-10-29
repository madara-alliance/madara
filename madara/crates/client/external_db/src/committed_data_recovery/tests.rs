use super::*;
use blockifier::execution::syscalls::committed_data::CommittedDataSet;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_gateway::committed_data::CommittedDataUpdate;

#[tokio::test]
#[ignore = "requires disposable MongoDB in the isolated recovery worker"]
async fn mongo_recovery_restores_bases_and_rejects_corruption() {
    let uri = std::env::var("RECOVERY_TEST_MONGO_URI").expect("isolated Mongo URI required");
    let client = mongodb::Client::with_uri_str(uri).await.unwrap();
    let db = client.database(&format!("recovery_faults_{}", uuid::Uuid::new_v4().simple()));
    let archive = CommittedDataArchive::new(&db, "transactions");
    let source = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let target = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let namespace = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    let entries = vec![(Felt::ONE, Felt::from(100u64)), (Felt::TWO, Felt::from(200u64))];
    let root = CommittedDataSet::new(entries.clone()).unwrap().root();
    source.update_committed_data(namespace, root, entries.clone()).await.unwrap();
    let base = source.journal_committed_data(Some(namespace), root, entries.clone()).await.unwrap();
    let base_record = source.db.get_recovery_record(&base).unwrap().unwrap();
    // Simulate interruption after uploading bytes but before committing the manifest.
    CommittedDataArchive::immutable(&archive.chunks, &base_record.manifest.chunks[0], base_record.chunks[0].clone())
        .await
        .unwrap();
    assert!(!archive.complete(&base).await.unwrap());
    assert!(archive.restore(&target, std::slice::from_ref(&base)).await.is_err());
    archive.upload(&base_record).await.unwrap();
    // The target represents an older DB containing only the first version.
    archive.restore(&target, std::slice::from_ref(&base)).await.unwrap();
    let changed = vec![(Felt::ONE, Felt::from(101u64)), (Felt::TWO, Felt::from(200u64))];
    let next_root = CommittedDataSet::new(changed).unwrap().root();
    let update = CommittedDataUpdate { base_root: Some(root), ..namespace };
    let patch = vec![(Felt::ONE, Felt::from(101u64))];
    source.update_committed_data(update, next_root, patch.clone()).await.unwrap();
    let next = source.journal_committed_data(Some(update), next_root, patch).await.unwrap();
    let next_record = source.db.get_recovery_record(&next).unwrap().unwrap();
    archive.upload(&next_record).await.unwrap();
    source.db.mark_recovery_exported(&base).unwrap();
    source.db.mark_recovery_exported(&next).unwrap();
    assert!(source.db.get_recovery_record(&base).unwrap().is_none());
    assert!(source.db.get_recovery_record(&next).unwrap().is_none());
    drop(source); // No live source or source DB access during restore.
    archive.restore(&target, std::slice::from_ref(&next)).await.unwrap();
    for (r, value) in [(root, 100u64), (next_root, 101u64)] {
        let witness = target.committed_data_witness(r, Felt::ONE).unwrap().unwrap();
        assert_eq!(witness.value, Felt::from(value));
        assert!(witness.verify());
    }
    assert_eq!(target.committed_data_witness(next_root, Felt::TWO).unwrap().unwrap().value, Felt::from(200u64));
    // Identical upload/restore is harmless, including after the local outbox marker disappeared.
    archive.upload(&next_record).await.unwrap();
    archive.restore(&target, std::slice::from_ref(&next)).await.unwrap();
    // Content hashes authenticate bytes, but do not substitute for validating the Merkle root.
    let mut wrong_root = base_record.clone();
    wrong_root.manifest.root = Felt::ZERO;
    archive.upload(&wrong_root).await.unwrap();
    assert!(archive.restore(&target, &[wrong_root.manifest.id().unwrap()]).await.is_err());
    let mut wrong_chain = base_record.clone();
    wrong_chain.manifest.chain_id = Felt::ZERO;
    archive.upload(&wrong_chain).await.unwrap();
    assert!(archive.restore(&target, &[wrong_chain.manifest.id().unwrap()]).await.is_err());
    let hash = &next_record.manifest.chunks[0];
    archive.chunks.delete_one(doc! {"_id":hash}).await.unwrap();
    assert!(archive.restore(&target, std::slice::from_ref(&next)).await.is_err());
    archive.upload(&next_record).await.unwrap();
    archive.chunks.update_one(doc! {"_id":hash}, doc! {"$set":{"payload":binary(vec![0,1,2])}}).await.unwrap();
    assert!(archive.download(&next).await.is_err());
    assert!(archive.upload(&next_record).await.is_err());
    // Repair the earlier corruption so only the missing parent can reject this upload.
    archive.chunks.delete_one(doc! {"_id":hash}).await.unwrap();
    archive.upload(&next_record).await.unwrap();
    // A manifest cannot complete while its exact parent is missing.
    archive.manifests.delete_one(doc! {"_id":&base}).await.unwrap();
    assert_eq!(archive.upload(&next_record).await.unwrap_err().to_string(), "Backup base is incomplete");
    // A corrupt completion marker must not authorize deleting the remaining local payload.
    archive
        .manifests
        .update_one(
            doc! {"_id":&next},
            doc! {"$set":{"payload":binary(serde_json::to_vec(&base_record.manifest).unwrap())}},
        )
        .await
        .unwrap();
    assert!(archive.complete(&next).await.is_err());
    db.drop().await.unwrap();
}

fn checkpoint() -> RecoveryRecord {
    let entries = vec![(Felt::ONE, Felt::from(100u64)), (Felt::TWO, Felt::from(200u64))];
    RecoveryRecord::encode(
        ChainConfig::madara_test().chain_id.to_felt(),
        Some(CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None }),
        CommittedDataSet::new(entries.clone()).unwrap().root(),
        None,
        entries,
    )
    .unwrap()
}

#[test]
fn recovery_parent_must_match_chain_namespace_and_root() {
    let base = checkpoint().manifest;
    let mut child = base.clone();
    child.update.as_mut().unwrap().base_root = Some(base.root);
    child.base = Some(base.id().unwrap());
    validate_parent(&child, &base).unwrap();
    for field in ["chain", "application", "dataset", "root", "namespace"] {
        let mut wrong = base.clone();
        match field {
            "chain" => wrong.chain_id = Felt::ZERO,
            "application" => wrong.update.as_mut().unwrap().application_id = Felt::ZERO,
            "dataset" => wrong.update.as_mut().unwrap().dataset_id = Felt::ZERO,
            "root" => wrong.root = Felt::ZERO,
            "namespace" => wrong.update = None,
            _ => unreachable!(),
        }
        assert!(validate_parent(&child, &wrong).is_err(), "{field}");
    }
    child.update = None;
    assert!(validate_parent(&child, &base).is_err());
}

#[test]
fn completion_manifest_requires_valid_bounded_bytes_and_exact_identity() {
    let manifest = checkpoint().manifest;
    let id = manifest.id().unwrap();
    let document = doc! {"payload":binary(serde_json::to_vec(&manifest).unwrap())};
    assert_eq!(CommittedDataArchive::decode_manifest(&document, &id).unwrap().id().unwrap(), id);
    assert!(CommittedDataArchive::decode_manifest(&document, &"0".repeat(64)).is_err());
    for bytes in [vec![0; 65537], b"invalid json".to_vec()] {
        assert!(CommittedDataArchive::decode_manifest(&doc! {"payload":binary(bytes)}, &id).is_err());
    }
    let mut invalid = manifest;
    invalid.version += 1;
    let document = doc! {"payload":binary(serde_json::to_vec(&invalid).unwrap())};
    assert!(CommittedDataArchive::decode_manifest(&document, &invalid.id().unwrap()).is_err());
    assert!(CommittedDataArchive::decode_manifest(&doc! {}, &id).is_err());
}

#[tokio::test]
async fn local_base_requires_manifest_tree_and_namespace_even_after_acknowledgement() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let record = checkpoint();
    let manifest = &record.manifest;
    let id = manifest.id().unwrap();
    assert!(!has_local_base(&backend, &id, manifest).unwrap());
    backend.db.store_recovery_record(&record).unwrap();
    assert!(!has_local_base(&backend, &id, manifest).unwrap());
    backend.import_committed_data_snapshot(manifest.root, record.decode().unwrap()).await.unwrap();
    assert!(!has_local_base(&backend, &id, manifest).unwrap());
    backend.update_committed_data(manifest.update.unwrap(), manifest.root, record.decode().unwrap()).await.unwrap();
    assert!(has_local_base(&backend, &id, manifest).unwrap());
    backend.db.mark_recovery_exported(&id).unwrap();
    assert!(has_local_base(&backend, &id, manifest).unwrap());
    assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
    let mut corrupt = manifest.clone();
    corrupt.root = Felt::ZERO;
    let db = backend.db.inner_db();
    let cf = db.cf_handle("committed_data_recovery").unwrap();
    let key = [b"m".as_slice(), id.as_bytes()].concat();
    db.put_cf(&cf, key, bincode::serialize(&corrupt).unwrap()).unwrap();
    assert!(has_local_base(&backend, &id, manifest).is_err());
}

#[tokio::test]
async fn restoring_records_preserves_parent_witnesses_and_acknowledges_payloads() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let base = checkpoint();
    let base_id = base.manifest.id().unwrap();
    restore_record(&backend, base_id.clone(), base.clone()).await.unwrap();
    let changed = vec![(Felt::ONE, Felt::from(101u64)), (Felt::TWO, Felt::from(200u64))];
    let root = CommittedDataSet::new(changed).unwrap().root();
    let patch = RecoveryRecord::encode(
        base.manifest.chain_id,
        Some(CommittedDataUpdate { base_root: Some(base.manifest.root), ..base.manifest.update.unwrap() }),
        root,
        Some(base_id.clone()),
        vec![(Felt::ONE, Felt::from(101u64))],
    )
    .unwrap();
    let patch_id = patch.manifest.id().unwrap();
    for _ in 0..2 {
        restore_record(&backend, patch_id.clone(), patch.clone()).await.unwrap();
    }
    for (root, value) in [(base.manifest.root, 100u64), (root, 101u64)] {
        let witness = backend.committed_data_witness(root, Felt::ONE).unwrap().unwrap();
        assert_eq!(witness.value, Felt::from(value));
        assert!(witness.verify());
        assert_eq!(backend.committed_data_witness(root, Felt::TWO).unwrap().unwrap().value, Felt::from(200u64));
    }
    for id in [base_id, patch_id] {
        assert_eq!(backend.db.get_recovery_manifest(&id).unwrap().unwrap().id().unwrap(), id);
        assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
    }
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 0);
}

#[tokio::test]
async fn invalid_records_never_publish_a_tree_or_recovery_manifest() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let record = checkpoint();
    let id = record.manifest.id().unwrap();
    assert!(restore_record(&backend, "0".repeat(64), record.clone()).await.is_err());
    for field in ["chain", "root", "chunk"] {
        let mut invalid = record.clone();
        match field {
            "chain" => invalid.manifest.chain_id = Felt::ZERO,
            "root" => invalid.manifest.root = Felt::ZERO,
            "chunk" => invalid.chunks[0][0] ^= 1,
            _ => unreachable!(),
        }
        let invalid_id = invalid.manifest.id().unwrap();
        assert!(restore_record(&backend, invalid_id.clone(), invalid).await.is_err(), "{field}");
        assert!(backend.db.get_recovery_manifest(&invalid_id).unwrap().is_none());
    }
    assert!(backend.db.list_committed_data_roots(None).unwrap().is_empty());
    assert!(backend.db.get_recovery_manifest(&id).unwrap().is_none());
    // Namespace-free full maps use the same authenticated restore path.
    let mut snapshot = record;
    snapshot.manifest.update = None;
    restore_record(&backend, snapshot.manifest.id().unwrap(), snapshot).await.unwrap();
    assert_eq!(backend.db.list_committed_data_roots(None).unwrap().len(), 1);
}

/// Restore consumes no upload capacity, even when the target's budget is smaller than the
/// archived checkpoint or its old upload queue is already above the newly configured budget.
#[rstest::rstest]
#[case::empty_queue(false)]
#[case::existing_queue(true)]
#[tokio::test]
async fn remote_restore_bypasses_upload_budget_and_survives_restart(#[case] queued: bool) {
    let directory = tempfile::tempdir().unwrap();
    let open = |budget| {
        MadaraBackend::open_rocksdb(
            directory.path(),
            ChainConfig::madara_test().into(),
            mc_db::MadaraBackendConfig {
                committed_data_retention: mc_db::committed_data_retention::CommittedDataRetentionConfig {
                    pending_backup_bytes: budget,
                    ..Default::default()
                },
                ..Default::default()
            },
            Default::default(),
            Default::default(),
        )
        .unwrap()
    };
    let pending;
    {
        let backend = open(1024 * 1024);
        if queued {
            let values = vec![(Felt::ZERO, Felt::ONE)];
            let root = CommittedDataSet::new(values.clone()).unwrap().root();
            backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
            backend.journal_committed_data(None, root, values).await.unwrap();
        }
        pending = backend.db.recovery_pending_bytes().unwrap();
    }
    let values: Vec<_> = (0..32u32).map(|key| (Felt::from(key), Felt::from(key + 100))).collect();
    let tree = CommittedDataSet::new(values.clone()).unwrap();
    let record =
        RecoveryRecord::encode(ChainConfig::madara_test().chain_id.to_felt(), None, tree.root(), None, values).unwrap();
    let id = record.manifest.id().unwrap();
    {
        let backend = open(1);
        restore_record(&backend, id.clone(), record.clone()).await.unwrap();
        restore_record(&backend, id.clone(), record).await.unwrap();
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), pending);
        assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
    }
    let backend = open(1);
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), pending);
    assert_eq!(
        backend
            .db
            .find_recovery_record(&mc_db::committed_data_recovery::namespace_key(None, tree.root()).unwrap())
            .unwrap(),
        Some(id.clone())
    );
    assert_eq!(backend.db.get_recovery_manifest(&id).unwrap().unwrap().root, tree.root());
    assert!(backend.committed_data_witness(tree.root(), Felt::ONE).unwrap().unwrap().verify());
}

/// Restoring an already uploaded local record acknowledges exactly that pending payload.
/// A conflicting namespace identity must never remove a different queued record.
#[tokio::test]
async fn remote_restore_acknowledges_matching_pending_payload_and_rejects_conflicts() {
    let backend = MadaraBackend::open_for_testing(mp_chain_config::ChainConfig::madara_test().into());
    let entries = vec![(Felt::ONE, Felt::MAX)];
    let root = CommittedDataSet::new(entries.clone()).unwrap().root();
    backend.import_committed_data_snapshot(root, entries.clone()).await.unwrap();
    let record = RecoveryRecord::encode(backend.chain_config().chain_id.to_felt(), None, root, None, entries).unwrap();
    let id = backend.db.store_recovery_record(&record).unwrap();
    let pending = backend.db.recovery_pending_bytes().unwrap();
    assert!(pending > 0);
    let mut conflict = record.manifest.clone();
    conflict.chain_id += Felt::ONE;
    assert!(backend.db.restore_recovery_manifest(&conflict).is_err());
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), pending);
    assert!(backend.db.get_recovery_record(&id).unwrap().is_some());
    restore_record(&backend, id.clone(), record.clone()).await.unwrap();
    restore_record(&backend, id.clone(), record).await.unwrap();
    assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 0);
    assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
    assert_eq!(backend.db.get_recovery_manifest(&id).unwrap().unwrap().id().unwrap(), id);
}

/// Recover a history longer than the local delta checkpoint interval using only Mongo backups.
/// A missing final chunk interrupts restore after valid ancestors commit; retry after restart
/// must resume without losing earlier roots, admitting an incomplete root, or needing upload space.
#[rstest::rstest]
#[tokio::test]
#[ignore = "requires disposable MongoDB in the isolated recovery worker"]
async fn mongo_historical_recovery_resumes_after_partial_restore_and_pruned_restart(
    #[values(0usize, 1, 33)] local_versions: usize,
    #[values(false, true)] prune_local: bool,
) {
    use mc_db::MadaraStorageWrite;
    let uri = std::env::var("RECOVERY_TEST_MONGO_URI").expect("isolated Mongo URI required");
    let client = mongodb::Client::with_uri_str(uri).await.unwrap();
    let mongo = client.database(&format!("history_matrix_{}", uuid::Uuid::new_v4().simple()));
    let archive = CommittedDataArchive::new(&mongo, "transactions");
    let directory = tempfile::tempdir().unwrap();
    let open = || {
        MadaraBackend::open_rocksdb(
            directory.path(),
            ChainConfig::madara_test().into(),
            mc_db::MadaraBackendConfig {
                committed_data_retention: mc_db::committed_data_retention::CommittedDataRetentionConfig {
                    cache_bytes: 0,
                    memory_blocks: 0,
                    pending_backup_bytes: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
            Default::default(),
            Default::default(),
        )
        .unwrap()
    };
    let namespace = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    let mut versions = Vec::new();
    let mut records = Vec::new();
    {
        let source = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let mut values = vec![(Felt::ZERO, Felt::ZERO), (Felt::ONE, Felt::MAX), (Felt::MAX, Felt::TWO)];
        for version in 0..67u64 {
            values[0].1 = Felt::from(version);
            let tree = CommittedDataSet::new(values.clone()).unwrap();
            let update = CommittedDataUpdate { base_root: versions.last().copied(), ..namespace };
            let payload = if version == 0 { values.clone() } else { vec![values[0]] };
            source.update_committed_data(update, tree.root(), payload.clone()).await.unwrap();
            let id = source.journal_committed_data(Some(update), tree.root(), payload).await.unwrap();
            let record = source.db.get_recovery_record(&id).unwrap().unwrap();
            archive.upload(&record).await.unwrap();
            source.db.mark_recovery_exported(&id).unwrap();
            assert!(source.db.get_recovery_record(&id).unwrap().is_none());
            versions.push(tree.root());
            records.push(record);
        }
        while source.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
    } // The original source and every queued local backup payload are now unavailable.
    {
        let target = open();
        for record in records.iter().take(local_versions) {
            restore_record(&target, record.manifest.id().unwrap(), record.clone()).await.unwrap();
        }
        if prune_local {
            while target.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
        }
    }
    let last = records.last().unwrap();
    let target_id = last.manifest.id().unwrap();
    archive.chunks.delete_one(doc! {"_id":&last.manifest.chunks[0]}).await.unwrap();
    {
        let target = open();
        let error = archive.restore(&target, std::slice::from_ref(&target_id)).await.unwrap_err();
        assert!(error.to_string().contains("Missing recovery chunk"));
        assert!(target.db.get_committed_data_count(*versions.last().unwrap()).unwrap().is_none());
        assert!(target.db.get_recovery_manifest(&target_id).unwrap().is_none());
        for (version, root) in versions.iter().take(versions.len() - 1).enumerate() {
            let witness = target.committed_data_witness(*root, Felt::ZERO).unwrap().unwrap();
            assert_eq!(witness.value, Felt::from(version));
            assert!(witness.verify());
        }
        assert_eq!(target.db.recovery_pending_bytes().unwrap(), 0);
    }
    archive.upload(last).await.unwrap();
    {
        let target = open();
        archive.restore(&target, std::slice::from_ref(&target_id)).await.unwrap();
        archive.restore(&target, std::slice::from_ref(&target_id)).await.unwrap();
        while target.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
        assert_eq!(target.db.recovery_pending_bytes().unwrap(), 0);
    }
    let target = open();
    for (version, root) in versions.iter().enumerate() {
        for (key, expected) in [(Felt::ZERO, Felt::from(version)), (Felt::ONE, Felt::MAX), (Felt::MAX, Felt::TWO)] {
            let witness = target.committed_data_witness(*root, key).unwrap().unwrap();
            assert_eq!(witness.value, expected);
            assert!(witness.verify());
        }
    }
    assert!(target.db.get_recovery_record(&target_id).unwrap().is_none());
    assert_eq!(target.db.get_recovery_manifest(&target_id).unwrap().unwrap().root, *versions.last().unwrap());
    mongo.drop().await.unwrap();
}

/// Archive corruption is an operator recovery event, not permission to serve unauthenticated data.
/// Preserve the damaged DB for diagnosis and restore into a fresh DB from verified remote records.
#[tokio::test]
#[ignore = "requires disposable MongoDB in the isolated recovery worker"]
async fn mongo_historical_recovery_rejects_damaged_local_archive_and_restores_fresh_database() {
    let uri = std::env::var("RECOVERY_TEST_MONGO_URI").expect("isolated Mongo URI required");
    let client = mongodb::Client::with_uri_str(uri).await.unwrap();
    let mongo = client.database(&format!("history_damage_{}", uuid::Uuid::new_v4().simple()));
    let archive = CommittedDataArchive::new(&mongo, "transactions");
    let record = checkpoint();
    let id = record.manifest.id().unwrap();
    archive.upload(&record).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let open = || {
        MadaraBackend::open_rocksdb(
            directory.path(),
            ChainConfig::madara_test().into(),
            Default::default(),
            Default::default(),
            Default::default(),
        )
        .unwrap()
    };
    let history_key = [b"cdh/v1/".as_slice(), &record.manifest.root.to_bytes_be()].concat();
    {
        let target = open();
        archive.restore(&target, std::slice::from_ref(&id)).await.unwrap();
        while target.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
        let cf = target.db.inner_db().cf_handle("meta").unwrap();
        target.db.inner_db().put_cf(&cf, &history_key, b"damaged archive").unwrap();
    }
    {
        let target = open();
        assert!(target.committed_data_witness(record.manifest.root, Felt::ONE).is_err());
        assert!(archive.restore(&target, std::slice::from_ref(&id)).await.is_err());
        let cf = target.db.inner_db().cf_handle("meta").unwrap();
        assert_eq!(target.db.inner_db().get_cf(&cf, &history_key).unwrap().unwrap(), b"damaged archive");
    }
    let fresh = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    archive.restore(&fresh, std::slice::from_ref(&id)).await.unwrap();
    while fresh.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
    for (key, value) in record.decode().unwrap() {
        let witness = fresh.committed_data_witness(record.manifest.root, key).unwrap().unwrap();
        assert_eq!(witness.value, value);
        assert!(witness.verify());
    }
    mongo.drop().await.unwrap();
}
