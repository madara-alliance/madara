//! Durable backup outbox, permanent manifests and transaction dependency identities.
//! All mutations use the records lock and a synchronous WAL; payload acknowledgement
//! removes only upload bytes, preserving identities needed for replay.
use super::{Column, RocksDBStorageInner};
use crate::{
    committed_data_recovery::{namespace_key, RecoveryManifest, RecoveryRecord},
    prelude::*,
};
use mp_convert::Felt;
use mp_gateway::committed_data::MAX_DATASETS_PER_TRANSACTION;
use mp_transactions::validated::ValidatedTransaction;
use rocksdb::{Direction, IteratorMode, WriteBatch, WriteOptions};
pub const COMMITTED_DATA_RECOVERY_COLUMN: Column = Column::new("committed_data_recovery").set_point_lookup();

// Prefixes are part of the persisted recovery-column layout. Identities survive payload ACK.
const NAMESPACE_PREFIX: u8 = b'n';
const RECORD_PREFIX: u8 = b'r';
const MANIFEST_PREFIX: u8 = b'm';
const PENDING_PREFIX: u8 = b'p';
const TRANSACTION_PREFIX: u8 = b't';
const PENDING_BYTES_KEY: &[u8] = b"bytes";

fn key(prefix: u8, id: &[u8]) -> Vec<u8> {
    let mut bytes = vec![prefix];
    bytes.extend_from_slice(id);
    bytes
}
fn durable() -> WriteOptions {
    let mut options = WriteOptions::default();
    options.set_sync(true);
    options.disable_wal(false);
    options
}
impl RocksDBStorageInner {
    /// Resolve a namespace/root identity, including records whose upload payload was acknowledged.
    pub(super) fn find_recovery_record(&self, namespace: &[u8]) -> Result<Option<String>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(NAMESPACE_PREFIX, namespace))?
            .map(|b| Ok(String::from_utf8(b)?))
            .transpose()
    }
    /// Read a pending upload payload; acknowledged records retain only their separate manifest.
    pub(super) fn get_recovery_record(&self, id: &str) -> Result<Option<RecoveryRecord>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(RECORD_PREFIX, id.as_bytes()))?
            .map(|b| Ok(super::deserialize(b)?))
            .transpose()
    }
    /// Read the permanent replay identity independently of temporary upload bytes.
    pub(super) fn get_recovery_manifest(&self, id: &str) -> Result<Option<RecoveryManifest>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(MANIFEST_PREFIX, id.as_bytes()))?
            .map(|b| Ok(super::deserialize(b)?))
            .transpose()
    }
    /// Return a bounded lexical batch of pending IDs for the asynchronous exporter.
    pub(super) fn pending_recovery_records(&self, limit: usize) -> Result<Vec<String>> {
        self.db
            .iterator_cf(
                &self.get_column(COMMITTED_DATA_RECOVERY_COLUMN),
                IteratorMode::From(&[PENDING_PREFIX], Direction::Forward),
            )
            .take_while(|r| r.as_ref().map_or(true, |(key, _)| key.starts_with(&[PENDING_PREFIX])))
            .take(limit)
            .map(|r| {
                let (key, _) = r?;
                Ok(String::from_utf8(key[1..].to_vec())?)
            })
            .collect()
    }
    /// Read dataset manifest IDs durably bound to a transaction admission.
    pub(super) fn recovery_dependencies(&self, hash: Felt) -> Result<Vec<String>> {
        Ok(self
            .db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(TRANSACTION_PREFIX, &hash.to_bytes_be()))?
            .map(super::deserialize)
            .transpose()?
            .unwrap_or_default())
    }
    /// Read serialized pending payload usage; this excludes permanent trees and manifests.
    pub(super) fn pending_recovery_bytes(&self) -> Result<u64> {
        Ok(self
            .db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), PENDING_BYTES_KEY)?
            .map(super::deserialize)
            .transpose()?
            .unwrap_or(0))
    }
    /// Idempotently journal an authenticated import, checking the actual compressed backlog under lock.
    pub(super) fn store_recovery_record(&self, record: &RecoveryRecord) -> Result<String> {
        let _lock = self.committed_data.records.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let namespace = namespace_key(record.manifest.update, record.manifest.root)?;
        if let Some(id) = self.find_recovery_record(&namespace)? {
            return Ok(id);
        }
        record.manifest.validate()?;
        if let Some(base) = &record.manifest.base {
            anyhow::ensure!(self.get_recovery_manifest(base)?.is_some(), "Recovery base missing");
        }
        let id = record.manifest.id()?;
        let bytes = super::serialize(record)?;
        let pending =
            self.pending_recovery_bytes()?.checked_add(bytes.len() as u64).context("Recovery outbox overflow")?;
        anyhow::ensure!(
            pending
                <= self
                    .committed_data
                    .config
                    .read()
                    .map_err(|_| anyhow::anyhow!("Committed-data config poisoned"))?
                    .pending_backup_bytes,
            "Committed-data backup backlog full; retry after export"
        );
        let cf = self.get_column(COMMITTED_DATA_RECOVERY_COLUMN);
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, key(MANIFEST_PREFIX, id.as_bytes()), super::serialize(&record.manifest)?);
        batch.put_cf(&cf, key(RECORD_PREFIX, id.as_bytes()), &bytes);
        batch.put_cf(&cf, key(NAMESPACE_PREFIX, &namespace), id.as_bytes());
        batch.put_cf(&cf, key(PENDING_PREFIX, id.as_bytes()), super::serialize(&(bytes.len() as u64))?);
        batch.put_cf(&cf, PENDING_BYTES_KEY, super::serialize(&pending)?);
        self.db.write_opt(batch, &durable())?;
        Ok(id)
    }
    /// After remote acknowledgement, atomically remove payload/index bytes and retain the manifest.
    pub(super) fn mark_recovery_exported(&self, id: &str) -> Result<()> {
        let _lock = self.committed_data.records.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let cf = self.get_column(COMMITTED_DATA_RECOVERY_COLUMN);
        anyhow::ensure!(self.get_recovery_manifest(id)?.is_some(), "Missing recovery manifest");
        let pending_key = key(PENDING_PREFIX, id.as_bytes());
        if let Some(bytes) = self.db.get_cf(&cf, &pending_key)? {
            let size: u64 = super::deserialize(bytes)?;
            let pending = self.pending_recovery_bytes()?.checked_sub(size).context("Recovery backlog underflow")?;
            let mut batch = WriteBatch::default();
            batch.delete_cf(&cf, pending_key);
            batch.delete_cf(&cf, key(RECORD_PREFIX, id.as_bytes()));
            batch.put_cf(&cf, PENDING_BYTES_KEY, super::serialize(&pending)?);
            self.db.write_opt(batch, &durable())?;
        }
        Ok(())
    }
    /// A single WAL-synced batch precedes visibility to block production and the Mongo exporter.
    pub(super) fn write_recovery_admission(
        &self,
        tx: &ValidatedTransaction,
        dependencies: &[String],
        export: bool,
    ) -> Result<()> {
        anyhow::ensure!(
            (1..=MAX_DATASETS_PER_TRANSACTION).contains(&dependencies.len()),
            "Invalid recovery dependency count"
        );
        for id in dependencies {
            anyhow::ensure!(self.get_recovery_manifest(id)?.is_some(), "Missing recovery dependency");
        }
        let cf = self.get_column(COMMITTED_DATA_RECOVERY_COLUMN);
        if let Some(existing) = self.db.get_cf(&cf, key(TRANSACTION_PREFIX, &tx.hash.to_bytes_be()))? {
            let previous: Vec<String> = super::deserialize(existing)?;
            anyhow::ensure!(previous == dependencies, "Transaction already bound to different datasets");
        }
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, key(TRANSACTION_PREFIX, &tx.hash.to_bytes_be()), super::serialize(&dependencies)?);
        batch.put_cf(
            &self.get_column(super::mempool::MEMPOOL_TRANSACTIONS_COLUMN),
            super::serialize(&tx.hash)?,
            super::serialize(tx)?,
        );
        if export {
            let id = super::external_outbox::ExternalOutboxId::new(tx.arrived_at);
            batch.put_cf(
                &self.get_column(super::external_outbox::MEMPOOL_EXTERNAL_OUTBOX_COLUMN),
                id.to_key_bytes(),
                super::serialize(tx)?,
            );
        }
        self.db.write_opt(batch, &durable())?;
        Ok(())
    }
}
