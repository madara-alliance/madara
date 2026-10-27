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
    let archive = RecoveryArchive::new(&db, "transactions");
    let source = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let target = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let namespace = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
    let entries = vec![(Felt::ONE, Felt::from(100u64)), (Felt::TWO, Felt::from(200u64))];
    let root = CommittedDataSet::new(entries.clone()).unwrap().root();
    source.update_committed_data(namespace, root, entries.clone()).await.unwrap();
    let base = source.journal_committed_data(Some(namespace), root, entries.clone()).await.unwrap();
    let base_record = source.db.get_recovery_record(&base).unwrap().unwrap();
    // Simulate interruption after uploading bytes but before committing the manifest.
    RecoveryArchive::immutable(&archive.chunks, &base_record.manifest.chunks[0], base_record.chunks[0].clone())
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
    // A manifest cannot complete while its exact parent is missing.
    archive.manifests.delete_one(doc! {"_id":&base}).await.unwrap();
    assert!(archive.upload(&next_record).await.is_err());
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
    assert_eq!(RecoveryArchive::decode_manifest(&document, &id).unwrap().id().unwrap(), id);
    assert!(RecoveryArchive::decode_manifest(&document, &"0".repeat(64)).is_err());
    for bytes in [vec![0; 65537], b"invalid json".to_vec()] {
        assert!(RecoveryArchive::decode_manifest(&doc! {"payload":binary(bytes)}, &id).is_err());
    }
    let mut invalid = manifest;
    invalid.version += 1;
    let document = doc! {"payload":binary(serde_json::to_vec(&invalid).unwrap())};
    assert!(RecoveryArchive::decode_manifest(&document, &invalid.id().unwrap()).is_err());
    assert!(RecoveryArchive::decode_manifest(&doc! {}, &id).is_err());
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
