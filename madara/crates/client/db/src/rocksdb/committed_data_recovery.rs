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
    pub(super) fn find_recovery_record(&self, namespace: &[u8]) -> Result<Option<String>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(b'n', namespace))?
            .map(|b| Ok(String::from_utf8(b)?))
            .transpose()
    }
    pub(super) fn get_recovery_record(&self, id: &str) -> Result<Option<RecoveryRecord>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(b'r', id.as_bytes()))?
            .map(|b| Ok(super::deserialize(b)?))
            .transpose()
    }
    pub(super) fn get_recovery_manifest(&self, id: &str) -> Result<Option<RecoveryManifest>> {
        self.db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(b'm', id.as_bytes()))?
            .map(|b| Ok(super::deserialize(b)?))
            .transpose()
    }
    pub(super) fn pending_recovery_records(&self, limit: usize) -> Result<Vec<String>> {
        self.db
            .iterator_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), IteratorMode::From(b"p", Direction::Forward))
            .take_while(|r| r.as_ref().map_or(true, |(k, _)| k.starts_with(b"p")))
            .take(limit)
            .map(|r| {
                let (k, _) = r?;
                Ok(String::from_utf8(k[1..].to_vec())?)
            })
            .collect()
    }
    pub(super) fn recovery_dependencies(&self, hash: Felt) -> Result<Vec<String>> {
        Ok(self
            .db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), key(b't', &hash.to_bytes_be()))?
            .map(super::deserialize)
            .transpose()?
            .unwrap_or_default())
    }
    pub(super) fn pending_recovery_bytes(&self) -> Result<u64> {
        Ok(self
            .db
            .get_cf(&self.get_column(COMMITTED_DATA_RECOVERY_COLUMN), b"bytes")?
            .map(super::deserialize)
            .transpose()?
            .unwrap_or(0))
    }
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
        batch.put_cf(&cf, key(b'm', id.as_bytes()), super::serialize(&record.manifest)?);
        batch.put_cf(&cf, key(b'r', id.as_bytes()), &bytes);
        batch.put_cf(&cf, key(b'n', &namespace), id.as_bytes());
        batch.put_cf(&cf, key(b'p', id.as_bytes()), super::serialize(&(bytes.len() as u64))?);
        batch.put_cf(&cf, b"bytes", super::serialize(&pending)?);
        self.db.write_opt(batch, &durable())?;
        Ok(id)
    }
    pub(super) fn mark_recovery_exported(&self, id: &str) -> Result<()> {
        let _lock = self.committed_data.records.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let cf = self.get_column(COMMITTED_DATA_RECOVERY_COLUMN);
        anyhow::ensure!(self.get_recovery_manifest(id)?.is_some(), "Missing recovery manifest");
        let k = key(b'p', id.as_bytes());
        if let Some(bytes) = self.db.get_cf(&cf, &k)? {
            let size: u64 = super::deserialize(bytes)?;
            let pending = self.pending_recovery_bytes()?.checked_sub(size).context("Recovery backlog underflow")?;
            let mut batch = WriteBatch::default();
            batch.delete_cf(&cf, k);
            batch.delete_cf(&cf, key(b'r', id.as_bytes()));
            batch.put_cf(&cf, b"bytes", super::serialize(&pending)?);
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
        if let Some(existing) = self.db.get_cf(&cf, key(b't', &tx.hash.to_bytes_be()))? {
            let previous: Vec<String> = super::deserialize(existing)?;
            anyhow::ensure!(previous == dependencies, "Transaction already bound to different datasets");
        }
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, key(b't', &tx.hash.to_bytes_be()), super::serialize(&dependencies)?);
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
