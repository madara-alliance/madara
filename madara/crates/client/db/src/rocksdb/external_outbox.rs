use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner},
};
use mp_transactions::validated::{TxTimestamp, ValidatedTransaction};
use rocksdb::{IteratorMode, ReadOptions};
use uuid::Uuid;

#[cfg(any(test, feature = "testing"))]
use std::sync::atomic::{AtomicBool, Ordering};

/// <arrived_at_ms u64 BE><uuid 16 bytes> => bincode(txn)
pub const MEMPOOL_EXTERNAL_OUTBOX_COLUMN: Column = Column::new("mempool_external_outbox").set_point_lookup();

const EXTERNAL_OUTBOX_KEY_LEN: usize = 24;

#[cfg(any(test, feature = "testing"))]
static FORCE_EXTERNAL_OUTBOX_WRITE_ERROR: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExternalOutboxId {
    pub arrived_at_ms: u64,
    pub uuid: [u8; 16],
}

impl ExternalOutboxId {
    fn new(arrived_at: TxTimestamp) -> Self {
        Self { arrived_at_ms: arrived_at.0, uuid: *Uuid::new_v4().as_bytes() }
    }

    fn to_key_bytes(self) -> [u8; EXTERNAL_OUTBOX_KEY_LEN] {
        let mut bytes = [0u8; EXTERNAL_OUTBOX_KEY_LEN];
        bytes[..8].copy_from_slice(&self.arrived_at_ms.to_be_bytes());
        bytes[8..].copy_from_slice(&self.uuid);
        bytes
    }

    fn from_key_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != EXTERNAL_OUTBOX_KEY_LEN {
            bail!("Invalid external outbox key length: {}", bytes.len());
        }
        let mut arrived = [0u8; 8];
        arrived.copy_from_slice(&bytes[..8]);
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&bytes[8..]);
        Ok(Self { arrived_at_ms: u64::from_be_bytes(arrived), uuid })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalOutboxEntry {
    pub id: ExternalOutboxId,
    pub tx: ValidatedTransaction,
}

#[cfg(any(test, feature = "testing"))]
pub fn set_external_outbox_write_failpoint(enabled: bool) {
    FORCE_EXTERNAL_OUTBOX_WRITE_ERROR.store(enabled, Ordering::Relaxed);
}

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self), fields(module = "ExternalOutboxDB"))]
    pub(super) fn iter_external_outbox(&self, limit: usize) -> impl Iterator<Item = Result<ExternalOutboxEntry>> + '_ {
        DBIterator::new_cf(
            &self.db,
            &self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN),
            ReadOptions::default(),
            IteratorMode::Start,
        )
        .into_iter_items(|(k, v)| (k.to_vec(), v.to_vec()))
        .map(|res| {
            let (key, value) = res?;
            let id = ExternalOutboxId::from_key_bytes(&key)?;
            let tx = super::deserialize::<ValidatedTransaction>(&value)?;
            Ok(ExternalOutboxEntry { id, tx })
        })
        .take(limit)
    }

    #[tracing::instrument(skip(self, tx), fields(module = "ExternalOutboxDB"))]
    pub(super) fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<ExternalOutboxId> {
        #[cfg(any(test, feature = "testing"))]
        if FORCE_EXTERNAL_OUTBOX_WRITE_ERROR.load(Ordering::Relaxed) {
            bail!("Forced external outbox write failure");
        }
        let col = self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN);
        let id = ExternalOutboxId::new(tx.arrived_at);
        self.db.put_cf(&col, id.to_key_bytes(), super::serialize(tx)?)?;
        Ok(id)
    }

    #[tracing::instrument(skip(self), fields(module = "ExternalOutboxDB"))]
    pub(super) fn delete_external_outbox(&self, id: ExternalOutboxId) -> Result<()> {
        let col = self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN);
        self.db.delete_cf_opt(&col, id.to_key_bytes(), &self.writeopts)?;
        Ok(())
    }
}
