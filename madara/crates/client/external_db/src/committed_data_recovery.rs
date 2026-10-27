//! Operator recovery and permanent, content-addressed committed-data backups.
//! Mongo access is trusted administration; this module does not expose a public restore RPC.
mod transactions;
pub use transactions::restore_transactions;
#[cfg(test)]
mod tests;

use anyhow::{ensure, Context, Result};
use mc_db::{
    committed_data_recovery::{
        digest, RecoveryManifest, RecoveryRecord, MAX_COMPRESSED_CHUNK_BYTES, MAX_MANIFEST_BYTES,
    },
    MadaraBackend, MadaraStorageRead, MadaraStorageWrite,
};
use mongodb::{
    bson::{doc, spec::BinarySubtype, Binary, Bson, Document},
    options::{Acknowledgment, CollectionOptions, ReadConcern, WriteConcern},
    Collection,
};
use mp_convert::ToFelt;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
/// Mongo collections containing immutable committed-data manifests and payload chunks.
pub struct CommittedDataArchive {
    manifests: Collection<Document>,
    chunks: Collection<Document>,
}
fn binary(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes })
}
impl CommittedDataArchive {
    /// Use majority reads and journaled majority writes before acknowledging backup completion.
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
    /// Validate a durable manifest before deleting the last local upload payload.
    pub async fn complete(&self, id: &str) -> Result<bool> {
        // Completion permits deleting the last local upload payload. A document at this
        // key is insufficient: authenticate its manifest before acknowledging the backup.
        let Some(document) = self.manifests.find_one(doc! {"_id":id}).await? else { return Ok(false) };
        Self::decode_manifest(&document, id)?;
        Ok(true)
    }
    /// Insert once; retries must match exact bytes and cannot overwrite conflicting content.
    async fn immutable(collection: &Collection<Document>, id: &str, bytes: Vec<u8>) -> Result<()> {
        let document = doc! {"_id":id,"payload":binary(bytes)};
        collection.update_one(doc! {"_id":id}, doc! {"$setOnInsert":document.clone()}).upsert(true).await?;
        ensure!(
            collection.find_one(doc! {"_id":id}).await?.as_ref() == Some(&document),
            "Conflicting immutable backup object"
        );
        Ok(())
    }
    /// Upload parent-dependent chunks first, then publish the complete manifest as the commit marker.
    pub async fn upload(&self, record: &RecoveryRecord) -> Result<()> {
        record.manifest.validate()?;
        if let Some(base) = &record.manifest.base {
            ensure!(self.complete(base).await?, "Backup base is incomplete");
        }
        ensure!(record.chunks.len() == record.manifest.chunks.len(), "Incomplete local backup record");
        for (hash, chunk) in record.manifest.chunks.iter().zip(&record.chunks) {
            ensure!(chunk.len() <= MAX_COMPRESSED_CHUNK_BYTES && digest(chunk) == *hash, "Invalid backup chunk");
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
        ensure!(bytes.len() <= MAX_MANIFEST_BYTES, "Oversized recovery manifest");
        let manifest: RecoveryManifest = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        ensure!(manifest.id()? == id, "Recovery manifest digest mismatch");
        Ok(manifest)
    }
    /// Fetch bounded, digest-verified chunks for one manifest; the importer verifies the MPT root.
    pub async fn download(&self, id: &str) -> Result<RecoveryRecord> {
        let manifest = self.manifest(id).await?;
        let mut chunks = Vec::new();
        for hash in &manifest.chunks {
            let doc = self.chunks.find_one(doc! {"_id":hash}).await?.context("Missing recovery chunk")?;
            let bytes = doc.get_binary_generic("payload")?;
            ensure!(
                bytes.len() <= MAX_COMPRESSED_CHUNK_BYTES && digest(bytes) == *hash,
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
            let path = self.restore_path(backend, target, &mut done).await?;
            for id in path.into_iter().rev() {
                restore_record(backend, id.clone(), self.download(&id).await?).await?;
                done.insert(id);
                restored += 1;
            }
        }
        Ok(restored)
    }

    /// Discover missing ancestors without retaining their payloads in memory.
    async fn restore_path(
        &self,
        backend: &MadaraBackend,
        target: &str,
        done: &mut HashSet<String>,
    ) -> Result<Vec<String>> {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(target.to_owned());
        while let Some(id) = next {
            if done.contains(&id) {
                break;
            }
            ensure!(seen.insert(id.clone()), "Cycle in recovery base chain");
            let manifest = self.manifest(&id).await?;
            ensure!(manifest.chain_id == backend.chain_config().chain_id.to_felt(), "Cross-chain recovery rejected");
            if let Some(base_id) = &manifest.base {
                validate_parent(&manifest, &self.manifest(base_id).await?)?;
            }
            // Reuse an exact local ancestor, but always audit the requested target's archived bytes.
            if has_local_base(backend, &id, &manifest)? && id != target {
                done.insert(id);
                break;
            }
            next = manifest.base;
            path.push(id);
        }
        Ok(path)
    }
}

fn validate_parent(manifest: &RecoveryManifest, base: &RecoveryManifest) -> Result<()> {
    let update = manifest.update.context("Base without namespace")?;
    let parent = base.update.context("Base without namespace")?;
    ensure!(
        base.chain_id == manifest.chain_id
            && parent.application_id == update.application_id
            && parent.dataset_id == update.dataset_id
            && Some(base.root) == update.base_root,
        "Wrong recovery base namespace/root"
    );
    Ok(())
}

fn has_local_base(backend: &MadaraBackend, id: &str, manifest: &RecoveryManifest) -> Result<bool> {
    let Some(local) = backend.db.get_recovery_manifest(id)? else { return Ok(false) };
    ensure!(local.id()? == id, "Corrupt local recovery manifest");
    let registered = match manifest.update {
        Some(update) => backend.db.has_committed_data_version(mp_gateway::committed_data::CommittedDataVersion {
            application_id: update.application_id,
            dataset_id: update.dataset_id,
            root: manifest.root,
        })?,
        None => true,
    };
    Ok(registered && backend.db.get_committed_data_count(manifest.root)?.is_some())
}

/// Decode and authenticate one record, import its tree, then retain only its local manifest.
async fn restore_record(backend: &Arc<MadaraBackend>, expected_id: String, record: RecoveryRecord) -> Result<()> {
    ensure!(record.manifest.chain_id == backend.chain_config().chain_id.to_felt(), "Cross-chain recovery rejected");
    ensure!(record.manifest.id()? == expected_id, "Recovery manifest digest mismatch");
    let (record, entries) = tokio::task::spawn_blocking(move || {
        let entries = record.decode()?;
        Ok::<_, anyhow::Error>((record, entries))
    })
    .await??;
    let manifest = &record.manifest;
    match manifest.update {
        Some(update) => backend.update_committed_data(update, manifest.root, entries).await?,
        None => backend.import_committed_data_snapshot(manifest.root, entries).await?,
    }
    let local = Arc::clone(backend);
    tokio::task::spawn_blocking(move || {
        ensure!(local.db.store_recovery_record(&record)? == expected_id, "Local recovery identity mismatch");
        local.db.mark_recovery_exported(&expected_id)
    })
    .await?
}
