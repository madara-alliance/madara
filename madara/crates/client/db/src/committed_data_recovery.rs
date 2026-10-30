//! Versioned private-import backups. Local payloads exist only until Mongo acknowledgement.
//! Durable manifests retain replay identities without duplicating historical values.
//! Each record contains only the submitted upserts, bound to an exact retained base.
use crate::{MadaraBackend, MadaraStorage, MadaraStorageRead};
use anyhow::{ensure, Context, Result};
use mp_convert::{Felt, ToFelt};
use mp_gateway::committed_data::CommittedDataUpdate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};

/// Version-1 wire-format chunk size, not an operational tuning knob. Readers validate
/// chunk counts against this value; changing it requires a versioned manifest format.
pub const CHUNK_ENTRIES: usize = 8192;
/// Use the same admission bound as the commitment implementation.
pub const MAX_ENTRIES: usize = blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
/// Default only: operators override `CommittedDataRetentionConfig::pending_backup_bytes`.
pub const DEFAULT_PENDING_BACKUP_BYTES: u64 = 512 * 1024 * 1024;
pub const RECOVERY_FORMAT_VERSION: u32 = 1;
const FELT_BYTES: usize = 32;
const ENTRY_BYTES: usize = 2 * FELT_BYTES;
const SHA256_HEX_BYTES: usize = 64;
/// Conservative Zstd framing/compression expansion allowance for a 512 KiB raw chunk.
const CHUNK_OVERHEAD_BYTES: usize = 1024;
pub const MAX_COMPRESSED_CHUNK_BYTES: usize = CHUNK_ENTRIES * ENTRY_BYTES + CHUNK_OVERHEAD_BYTES;
/// Bounds manifest allocation and reserves room for identity/catalog serialization in preflight.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const BACKUP_COMPRESSION_LEVEL: i32 = 3;

/// SHA-256 identity of exact stored bytes; hexadecimal form is used in local and Mongo keys.
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Content-addressed dependency identity retained permanently after upload payload removal.
pub struct RecoveryManifest {
    pub version: u32,
    pub chain_id: Felt,
    pub update: Option<CommittedDataUpdate>,
    pub root: Felt,
    pub base: Option<String>,
    pub entries: u32,
    pub chunks: Vec<String>,
}
impl RecoveryManifest {
    /// Hash the canonical manifest serialization, binding chain, namespace, root and exact parents.
    pub fn id(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(self)?))
    }
    /// Validate version-1 structural bounds before any payload allocation or dependency traversal.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == RECOVERY_FORMAT_VERSION, "Unsupported recovery version");
        ensure!((1..=MAX_ENTRIES).contains(&(self.entries as usize)), "Invalid recovery entry count");
        ensure!(self.chunks.len() == (self.entries as usize).div_ceil(CHUNK_ENTRIES), "Invalid chunk count");
        ensure!(self.update.and_then(|u| u.base_root).is_some() == self.base.is_some(), "Missing or unexpected base");
        for id in self.chunks.iter().chain(self.base.iter()) {
            ensure!(
                id.len() == SHA256_HEX_BYTES && id.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid recovery digest"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// One submitted full map or patch, chunked as sorted `(key, value)` pairs and Zstd-compressed.
/// Decoding checks transport integrity; the importer separately authenticates the resulting MPT root.
pub struct RecoveryRecord {
    pub manifest: RecoveryManifest,
    pub chunks: Vec<Vec<u8>>,
}
impl RecoveryRecord {
    /// Sort submitted upserts, reject duplicates, and independently compress bounded pair chunks.
    pub fn encode(
        chain_id: Felt,
        update: Option<CommittedDataUpdate>,
        root: Felt,
        base: Option<String>,
        mut entries: Vec<(Felt, Felt)>,
    ) -> Result<Self> {
        ensure!((1..=MAX_ENTRIES).contains(&entries.len()), "Invalid recovery entry count");
        entries.sort_unstable_by_key(|e| e.0);
        ensure!(entries.windows(2).all(|w| w[0].0 < w[1].0), "Duplicate recovery key");
        let mut chunks = Vec::new();
        for part in entries.chunks(CHUNK_ENTRIES) {
            let mut raw = Vec::with_capacity(part.len() * ENTRY_BYTES);
            for (key, value) in part {
                raw.extend_from_slice(&key.to_bytes_be());
                raw.extend_from_slice(&value.to_bytes_be());
            }
            chunks.push(zstd::stream::encode_all(raw.as_slice(), BACKUP_COMPRESSION_LEVEL)?);
        }
        let manifest = RecoveryManifest {
            version: RECOVERY_FORMAT_VERSION,
            chain_id,
            update,
            root,
            base,
            entries: entries.len() as u32,
            chunks: chunks.iter().map(|c| digest(c)).collect(),
        };
        manifest.validate()?;
        Ok(Self { manifest, chunks })
    }
    /// Verify digests, bound decompression, and reject noncanonical felts/unsorted keys.
    pub fn decode(&self) -> Result<Vec<(Felt, Felt)>> {
        self.manifest.validate()?;
        ensure!(self.chunks.len() == self.manifest.chunks.len(), "Incomplete recovery record");
        let mut entries = Vec::with_capacity(self.manifest.entries as usize);
        for (chunk, hash) in self.chunks.iter().zip(&self.manifest.chunks) {
            ensure!(chunk.len() <= MAX_COMPRESSED_CHUNK_BYTES && digest(chunk) == *hash, "Corrupt recovery chunk");
            let mut bytes = Vec::new();
            zstd::stream::read::Decoder::new(chunk.as_slice())?
                .take((CHUNK_ENTRIES * ENTRY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            ensure!(
                !bytes.is_empty() && bytes.len() <= CHUNK_ENTRIES * ENTRY_BYTES && bytes.len() % ENTRY_BYTES == 0,
                "Invalid decoded chunk length"
            );
            for pair in bytes.chunks_exact(ENTRY_BYTES) {
                let key_bytes: [u8; 32] = pair[..FELT_BYTES].try_into()?;
                let value_bytes: [u8; 32] = pair[FELT_BYTES..].try_into()?;
                let key = Felt::from_bytes_be(&key_bytes);
                let value = Felt::from_bytes_be(&value_bytes);
                ensure!(key.to_bytes_be() == key_bytes && value.to_bytes_be() == value_bytes, "Noncanonical felt");
                entries.push((key, value));
            }
        }
        ensure!(entries.len() == self.manifest.entries as usize, "Recovery count mismatch");
        ensure!(entries.windows(2).all(|w| w[0].0 < w[1].0), "Unsorted recovery keys");
        Ok(entries)
    }
}

/// Key a namespace/root identity independently of the explicit base chosen for a patch.
pub fn namespace_key(update: Option<CommittedDataUpdate>, root: Felt) -> Result<Vec<u8>> {
    // base_root is not part of a version's identity; branches keep their explicit dependency in the manifest.
    Ok(serde_json::to_vec(&(update.map(|u| (u.application_id, u.dataset_id)), root))?)
}
impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Conservative preflight before importing a new tree. The atomic journal write rechecks
    /// actual compressed bytes; this prevents rejected imports growing trees during a full backlog.
    pub fn check_recovery_capacity(
        &self,
        update: Option<CommittedDataUpdate>,
        root: Felt,
        entries: usize,
    ) -> Result<()> {
        if self.db.find_recovery_record(&namespace_key(update, root)?)?.is_some() {
            return Ok(());
        }
        ensure!((1..=MAX_ENTRIES).contains(&entries), "Invalid recovery entry count");
        let worst_case = (entries * ENTRY_BYTES
            + entries.div_ceil(CHUNK_ENTRIES) * CHUNK_OVERHEAD_BYTES
            + MAX_MANIFEST_BYTES) as u64;
        ensure!(
            self.db.recovery_pending_bytes()?.saturating_add(worst_case)
                <= self.config.committed_data_retention.pending_backup_bytes,
            "Committed-data backup backlog full; retry after export"
        );
        Ok(())
    }

    /// Resolve the durable manifest needed by transaction admission or a child backup.
    pub fn recovery_reference(&self, update: Option<CommittedDataUpdate>, root: Felt) -> Result<String> {
        self.db
            .find_recovery_record(&namespace_key(update, root)?)?
            .context("Dataset has no recovery record; import a checkpoint first")
    }
}
impl<D: MadaraStorage> MadaraBackend<D> {
    /// Call only after the normal authenticated import and Merkle-root verification succeeds.
    pub async fn journal_committed_data(
        self: &Arc<Self>,
        update: Option<CommittedDataUpdate>,
        root: Felt,
        entries: Vec<(Felt, Felt)>,
    ) -> Result<String> {
        let backend = self.clone();
        let id = tokio::task::spawn_blocking(move || {
            let key = namespace_key(update, root)?;
            if let Some(id) = backend.db.find_recovery_record(&key)? {
                return Ok(id);
            }
            let base =
                update.and_then(|u| u.base_root).map(|base| backend.recovery_reference(update, base)).transpose()?;
            let record =
                RecoveryRecord::encode(backend.chain_config().chain_id.to_felt(), update, root, base, entries)?;
            backend.db.store_recovery_record(&record)
        })
        .await??;
        self.request_committed_data_cleanup();
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod crash;

    #[tokio::test]
    async fn acknowledged_payload_removed_but_parent_and_replay_references_survive() {
        use crate::MadaraStorageWrite;
        use mp_gateway::committed_data::CommittedDataUpdate;
        let backend = MadaraBackend::open_for_testing(mp_chain_config::ChainConfig::madara_test().into());
        let update = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
        let record =
            RecoveryRecord::encode(Felt::ONE, Some(update), Felt::THREE, None, vec![(Felt::ONE, Felt::TWO)]).unwrap();
        let id = backend.db.store_recovery_record(&record).unwrap();
        assert!(backend.db.recovery_pending_bytes().unwrap() > 0);
        backend.db.mark_recovery_exported(&id).unwrap();
        backend.db.mark_recovery_exported(&id).unwrap();
        assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
        assert_eq!(backend.db.get_recovery_manifest(&id).unwrap().unwrap().id().unwrap(), id);
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 0);
        assert_eq!(backend.db.store_recovery_record(&record).unwrap(), id);
        assert!(backend.db.get_recovery_record(&id).unwrap().is_none());
        let child = RecoveryRecord::encode(
            Felt::ONE,
            Some(CommittedDataUpdate { base_root: Some(Felt::THREE), ..update }),
            Felt::from(4u64),
            Some(id.clone()),
            vec![(Felt::ONE, Felt::THREE)],
        )
        .unwrap();
        let child_id = backend.db.store_recovery_record(&child).unwrap();
        assert!(backend.db.get_recovery_record(&child_id).unwrap().is_some());
        let sender = crate::test_utils::devnet_account_address();
        let tx = crate::test_utils::validated_transaction(
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V3(crate::test_utils::invoke_v3(
                sender,
                Felt::ZERO,
            ))),
            Felt::THREE,
            sender,
            None,
        );
        backend.db.write_recovery_admission(&tx, std::slice::from_ref(&id), true).unwrap();
        assert_eq!(backend.db.recovery_dependencies(Felt::THREE).unwrap(), vec![id]);
        assert_eq!(backend.get_saved_mempool_transactions().count(), 1);
        assert!(backend.db.mark_recovery_exported("missing").is_err());
    }

    #[tokio::test]
    async fn backlog_preflight_rejects_new_tree_but_allows_idempotent_retry() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let backend = MadaraBackend::open_for_testing(mp_chain_config::ChainConfig::madara_test().into());
        let entries = vec![(Felt::ONE, Felt::TWO)];
        let root = CommittedDataSet::new(entries.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, entries.clone()).await.unwrap();
        backend.journal_committed_data(None, root, entries).await.unwrap();
        crate::test_utils::set_recovery_pending_bytes(&backend, DEFAULT_PENDING_BACKUP_BYTES);
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), DEFAULT_PENDING_BACKUP_BYTES);
        assert_eq!(
            backend.check_recovery_capacity(None, Felt::ZERO, 1).unwrap_err().to_string(),
            "Committed-data backup backlog full; retry after export"
        );
        assert!(backend.check_recovery_capacity(None, root, 1).is_ok());
    }

    #[test]
    fn configured_backup_budget_checks_preflight_and_atomic_write() {
        use crate::{MadaraBackendConfig, MadaraStorageWrite};
        let dir = tempfile::tempdir().unwrap();
        let config = MadaraBackendConfig {
            committed_data_retention: crate::committed_data_retention::CommittedDataRetentionConfig {
                pending_backup_bytes: 128 * 1024,
                ..Default::default()
            },
            ..Default::default()
        };
        let backend = MadaraBackend::open_rocksdb(
            dir.path(),
            mp_chain_config::ChainConfig::madara_test().into(),
            config,
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert!(backend.check_recovery_capacity(None, Felt::THREE, 1).is_ok());
        assert!(backend.check_recovery_capacity(None, Felt::THREE, 4096).is_err());
        let record = RecoveryRecord::encode(Felt::ONE, None, Felt::THREE, None, vec![(Felt::ONE, Felt::TWO)]).unwrap();
        crate::test_utils::set_recovery_pending_bytes(&backend, 128 * 1024 - 1);
        assert!(backend.db.store_recovery_record(&record).is_err());
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 128 * 1024 - 1);
        assert!(backend.db.get_recovery_record(&record.manifest.id().unwrap()).unwrap().is_none());
        crate::test_utils::set_recovery_pending_bytes(&backend, 0);
        let id = backend.db.store_recovery_record(&record).unwrap();
        assert!(backend.db.recovery_pending_bytes().unwrap() < 128 * 1024);
        backend.db.mark_recovery_exported(&id).unwrap();
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), 0);
    }

    #[test]
    fn codec_roundtrip_and_corruption() {
        let entries: Vec<_> = (0..20_000).map(|i| (Felt::from(i as u64), Felt::from((i * 11) as u64))).collect();
        let mut record = RecoveryRecord::encode(Felt::ONE, None, Felt::TWO, None, entries.clone()).unwrap();
        assert_eq!(record.decode().unwrap(), entries);
        record.chunks[0][0] ^= 1;
        assert!(record.decode().is_err());
    }
    #[test]
    fn rejects_incomplete_and_duplicate_inputs() {
        assert!(RecoveryRecord::encode(Felt::ONE, None, Felt::TWO, None, vec![(Felt::ONE, Felt::ZERO); 2]).is_err());
        let mut record =
            RecoveryRecord::encode(Felt::ONE, None, Felt::TWO, None, vec![(Felt::ONE, Felt::ZERO)]).unwrap();
        record.chunks.clear();
        assert!(record.decode().is_err());
    }
}
