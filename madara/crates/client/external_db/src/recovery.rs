//! Operator recovery and permanent, content-addressed committed-data backups.
//! Mongo access is trusted administration; this module does not expose a public restore RPC.
use anyhow::{ensure, Context, Result};
use bincode::Options;
use mc_db::{
    committed_data_recovery::{digest, RecoveryManifest, RecoveryRecord, CHUNK_ENTRIES},
    MadaraBackend, MadaraStorageRead, MadaraStorageWrite,
};
use mongodb::{
    bson::{doc, spec::BinarySubtype, Binary, Bson, Document},
    options::{Acknowledgment, CollectionOptions, ReadConcern, WriteConcern},
    Collection,
};
use mp_convert::{Felt, FeltHexDisplay, ToFelt};
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
pub struct RecoveryArchive {
    manifests: Collection<Document>,
    chunks: Collection<Document>,
}
fn binary(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes })
}
impl RecoveryArchive {
    pub fn new(database: &mongodb::Database, prefix: &str) -> Self {
        let opts = CollectionOptions::builder()
            // A retry must not mistake a primary-only visible manifest for durable completion
            // and discard the last local upload payload before replication acknowledges it.
            .read_concern(ReadConcern::majority())
            .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).journal(true).build())
            .build();
        Self {
            manifests: database.collection_with_options(&format!("{prefix}_committed_manifests_v1"), opts.clone()),
            chunks: database.collection_with_options(&format!("{prefix}_committed_chunks_v1"), opts),
        }
    }
    pub async fn complete(&self, id: &str) -> Result<bool> {
        // Completion permits deleting the last local upload payload. A document at this
        // key is insufficient: authenticate its manifest before acknowledging the backup.
        let Some(document) = self.manifests.find_one(doc! {"_id":id}).await? else { return Ok(false) };
        Self::decode_manifest(&document, id)?;
        Ok(true)
    }
    async fn immutable(collection: &Collection<Document>, id: &str, bytes: Vec<u8>) -> Result<()> {
        let document = doc! {"_id":id,"payload":binary(bytes)};
        collection.update_one(doc! {"_id":id}, doc! {"$setOnInsert":document.clone()}).upsert(true).await?;
        ensure!(
            collection.find_one(doc! {"_id":id}).await?.as_ref() == Some(&document),
            "Conflicting immutable backup object"
        );
        Ok(())
    }
    pub async fn upload(&self, record: &RecoveryRecord) -> Result<()> {
        record.manifest.validate()?;
        if let Some(base) = &record.manifest.base {
            ensure!(self.complete(base).await?, "Backup base is incomplete");
        }
        ensure!(record.chunks.len() == record.manifest.chunks.len(), "Incomplete local backup record");
        for (hash, chunk) in record.manifest.chunks.iter().zip(&record.chunks) {
            ensure!(chunk.len() <= CHUNK_ENTRIES * 64 + 1024 && digest(chunk) == *hash, "Invalid backup chunk");
            Self::immutable(&self.chunks, hash, chunk.clone()).await?;
        }
        // Only complete manifests are visible, after majority+journal acknowledgement of every chunk.
        Self::immutable(&self.manifests, &record.manifest.id()?, serde_json::to_vec(&record.manifest)?).await
    }
    async fn manifest(&self, id: &str) -> Result<RecoveryManifest> {
        let doc = self.manifests.find_one(doc! {"_id":id}).await?.context("Missing complete recovery manifest")?;
        Self::decode_manifest(&doc, id)
    }
    fn decode_manifest(doc: &Document, id: &str) -> Result<RecoveryManifest> {
        let bytes = doc.get_binary_generic("payload")?;
        ensure!(bytes.len() <= 65536, "Oversized recovery manifest");
        let manifest: RecoveryManifest = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        ensure!(manifest.id()? == id, "Recovery manifest digest mismatch");
        Ok(manifest)
    }
    pub async fn download(&self, id: &str) -> Result<RecoveryRecord> {
        let manifest = self.manifest(id).await?;
        let mut chunks = Vec::new();
        for hash in &manifest.chunks {
            let doc = self.chunks.find_one(doc! {"_id":hash}).await?.context("Missing recovery chunk")?;
            let bytes = doc.get_binary_generic("payload")?;
            ensure!(
                bytes.len() <= CHUNK_ENTRIES * 64 + 1024 && digest(bytes) == *hash,
                "Recovery chunk digest mismatch"
            );
            chunks.push(bytes.to_vec());
        }
        Ok(RecoveryRecord { manifest, chunks })
    }
    /// Explicit operator action before transaction replay. Never trusts old completion markers.
    /// Walk exact parent chains iteratively so multi-day journals do not overflow the call stack.
    pub async fn restore(&self, backend: &Arc<MadaraBackend>, targets: &[String]) -> Result<usize> {
        let mut restored = 0;
        let mut done = HashSet::new();
        for target in targets {
            let mut path = Vec::new();
            let mut seen = HashSet::new();
            let mut next = Some(target.clone());
            while let Some(id) = next {
                if done.contains(&id) {
                    break;
                }
                ensure!(seen.insert(id.clone()), "Cycle in recovery base chain");
                let manifest = self.manifest(&id).await?;
                ensure!(
                    manifest.chain_id == backend.chain_config().chain_id.to_felt(),
                    "Cross-chain recovery rejected"
                );
                if let Some(base_id) = &manifest.base {
                    let base = self.manifest(base_id).await?;
                    let update = manifest.update.context("Base without namespace")?;
                    let parent = base.update.context("Base without namespace")?;
                    ensure!(
                        base.chain_id == manifest.chain_id
                            && parent.application_id == update.application_id
                            && parent.dataset_id == update.dataset_id
                            && Some(base.root) == update.base_root,
                        "Wrong recovery base namespace/root"
                    );
                }
                // An older restored DB may already contain this exact verified base. Stop here;
                // replaying the entire permanent journal would turn a short catch-up into full history.
                if let Some(local) = backend.db.get_recovery_manifest(&id)? {
                    ensure!(local.id()? == id, "Corrupt local recovery manifest");
                    let registered = match manifest.update {
                        Some(update) => {
                            backend.db.has_committed_data_version(mp_gateway::committed_data::CommittedDataVersion {
                                application_id: update.application_id,
                                dataset_id: update.dataset_id,
                                root: manifest.root,
                            })?
                        }
                        None => true,
                    };
                    if registered && backend.db.get_committed_data_count(manifest.root)?.is_some() {
                        // Still read/verify archived bytes for the requested target; a local cache
                        // must not conceal a corrupt external backup during a recovery audit.
                        if &id != target {
                            done.insert(id);
                            break;
                        }
                    }
                }
                next = manifest.base.clone();
                path.push(id);
            }
            for id in path.into_iter().rev() {
                let record = self.download(&id).await?;
                let (record, entries) = tokio::task::spawn_blocking(move || {
                    let entries = record.decode()?;
                    Ok::<_, anyhow::Error>((record, entries))
                })
                .await??;
                let m = &record.manifest;
                match m.update {
                    Some(update) => backend.update_committed_data(update, m.root, entries).await?,
                    None => backend.import_committed_data_snapshot(m.root, entries).await?,
                }
                let local = Arc::clone(backend);
                let expected_id = id.clone();
                tokio::task::spawn_blocking(move || {
                    ensure!(
                        local.db.store_recovery_record(&record)? == expected_id,
                        "Local recovery identity mismatch"
                    );
                    local.db.mark_recovery_exported(&expected_id)
                })
                .await??;
                done.insert(id);
                restored += 1;
            }
        }
        Ok(restored)
    }
}

fn decode_recovery_transaction(bytes: &[u8]) -> Result<ValidatedTransaction> {
    let mut tx: ValidatedTransaction = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(16 * 1024 * 1024)
        .reject_trailing_bytes()
        .deserialize(bytes)?;
    // Resubmissions have different admission times. Recovery assigns a fresh time after comparison.
    tx.arrived_at = TxTimestamp::UNIX_EPOCH;
    Ok(tx)
}

/// Stage only explicitly selected backed-up committed-data transactions. Mempool startup
/// reconciles them against the restored chain, including nonces already executed there.
/// A fresh local arrival timestamp avoids silently dropping disaster-recovery work by TTL;
/// Mongo retains the original arrival timestamp for audit and replay ordering.
pub async fn restore_transactions(
    backend: &Arc<MadaraBackend>,
    config: &crate::config::ExternalDbConfig,
    hashes: &[Felt],
) -> Result<usize> {
    use futures::TryStreamExt;
    let client = crate::mongodb::MongoClient::new(config).await?;
    let archive = RecoveryArchive::new(&client.client().database(&config.database_name), &config.collection_name);
    let chain_id = backend.chain_config().chain_id.to_string();
    let mut restored = 0;
    for hash in hashes {
        let mut cursor=client.collection().find(doc! {"tx_hash":format!("{}",hash.hex_display()),"chain_id":&chain_id,"committed_data.0":{"$exists":true}}).sort(doc!{"arrived_at":1,"_id":1}).await?;
        let document = cursor.try_next().await?.context("Missing recoverable transaction in Mongo archive")?;
        let mut tx = decode_recovery_transaction(&document.raw_transaction.bytes)?;
        while let Some(duplicate) = cursor.try_next().await? {
            ensure!(
                decode_recovery_transaction(&duplicate.raw_transaction.bytes)? == tx
                    && duplicate.committed_data == document.committed_data,
                "Conflicting transaction archive records"
            );
        }
        ensure!((1..=4).contains(&document.committed_data.len()), "Invalid recovery dependency count");
        ensure!(
            tx.hash == *hash
                && tx.transaction.compute_hash(
                    backend.chain_config().chain_id.to_felt(),
                    backend.chain_config().latest_protocol_version,
                    false
                ) == *hash,
            "Recovery transaction hash mismatch"
        );
        archive.restore(backend, &document.committed_data).await?;
        tx.arrived_at = TxTimestamp::now();
        backend.db.write_recovery_admission(&tx, &document.committed_data, false)?;
        restored += 1;
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    use mp_chain_config::ChainConfig;
    use mp_gateway::committed_data::CommittedDataUpdate;
    use mp_transactions::{InvokeTransaction, Transaction};

    #[test]
    fn recovery_comparison_ignores_only_arrival_time() {
        let sender = mc_db::test_utils::devnet_account_address();
        let mut original = mc_db::test_utils::validated_transaction(
            Transaction::Invoke(InvokeTransaction::V3(mc_db::test_utils::invoke_v3(sender, Felt::ONE))),
            Felt::ONE,
            sender,
            None,
        );
        original.arrived_at = TxTimestamp(100);
        let original_bytes = bincode::serialize(&original).unwrap();
        let expected = decode_recovery_transaction(&original_bytes).unwrap();
        let mut resubmitted = original.clone();
        resubmitted.arrived_at = TxTimestamp(200);
        let resubmitted_bytes = bincode::serialize(&resubmitted).unwrap();
        assert_ne!(original_bytes, resubmitted_bytes);
        assert_eq!(expected, decode_recovery_transaction(&resubmitted_bytes).unwrap());

        let mut changed_transaction = resubmitted.clone();
        changed_transaction.transaction =
            Transaction::Invoke(InvokeTransaction::V3(mc_db::test_utils::invoke_v3(sender, Felt::TWO)));
        let mut changed_fee = resubmitted.clone();
        changed_fee.charge_fee = !changed_fee.charge_fee;
        let mut changed_l1_fee = resubmitted.clone();
        changed_l1_fee.paid_fee_on_l1 = Some(1);
        let mut changed_address = resubmitted.clone();
        changed_address.contract_address = Felt::ZERO;
        let mut changed_hash = resubmitted;
        changed_hash.hash = Felt::TWO;
        for changed in [changed_transaction, changed_fee, changed_l1_fee, changed_address, changed_hash] {
            assert_ne!(expected, decode_recovery_transaction(&bincode::serialize(&changed).unwrap()).unwrap());
        }

        assert!(decode_recovery_transaction(&original_bytes[..original_bytes.len() - 1]).is_err());
        let mut trailing = original_bytes;
        trailing.push(0);
        assert!(decode_recovery_transaction(&trailing).is_err());
    }

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
}
