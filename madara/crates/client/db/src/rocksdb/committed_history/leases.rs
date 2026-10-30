//! Durable idle leases for rebuilt proof indexes, independent of execution/block retention.
//!
//! A root owns its complete reachable index through the existing node reference counts. Leased
//! roots leave the block-age queue; an expiry-ordered index returns them to it after idle expiry.
//! Nodes, root pointer and lease are published in one synced batch. Historical values, catalog
//! labels, import checkpoint and last execution block are never changed by a historical read.
use super::*;
use crate::persistent_committed_data::{Tree, NEXT_KEY};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const LEASE: &[u8] = b"cdh/lease/";
const EXPIRY: &[u8] = b"cdh/expiry/";

pub(super) fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn expiry_key(expires: u64, root: Felt) -> Vec<u8> {
    [EXPIRY, &expires.to_be_bytes(), &root.to_bytes_be()].concat()
}

impl RocksDBStorage {
    /// Return a stable read view, reconstructing a pruned index once and renewing its disk lease.
    /// Ordinary retained roots take the snapshot-only fast path. A snapshot protects in-flight
    /// readers even if the lease expires and maintenance deletes the live index concurrently.
    pub(in crate::rocksdb) fn historical_proof_snapshot(&self, root: Felt) -> Result<SnapshotWithDBArc> {
        self.historical_proof_snapshot_at(root, unix_seconds()?)
    }

    pub(super) fn historical_proof_snapshot_at(&self, root: Felt, now: u64) -> Result<SnapshotWithDBArc> {
        let idle_secs = self
            .inner
            .committed_data
            .config
            .read()
            .map_err(|_| anyhow::anyhow!("Committed-data config poisoned"))?
            .rebuilt_proof_idle_secs;
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
        if idle_secs == 0 {
            return Ok(snapshot);
        }
        let cf = self.inner.get_column(meta::META_COLUMN);
        let Some(metadata) = snapshot.get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))? else {
            return Ok(snapshot);
        };
        if persistent_id(&metadata)? != 0 && snapshot.get_cf(&cf, root_key(LEASE, root))?.is_none() {
            return Ok(snapshot);
        }
        drop(snapshot);
        // Share the existing tree writer lock: no lock inversion with cold mutation/recovery.
        // A concurrent miss rechecks the pointer after this lock, avoiding a second rebuild.
        let _writer =
            self.inner.committed_data.tree.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let mut metadata = self.inner.db.get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?.context("Missing root")?;
        let mut batch = WriteBatch::default();
        if persistent_id(&metadata)? == 0 {
            let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
            let mut entries = historical_values(&snapshot, root)?;
            anyhow::ensure!(entries.len() == count(&metadata)?, "Historical value count mismatch");
            let first = self.next_committed_node_id()?;
            let mut tree = Tree::new(first, MAX_COMMITTED_DATA_VALUES as u32, |_| Ok(None));
            let id = tree.build(&mut entries)?;
            anyhow::ensure!(tree.root(id)?.0 == root, "Reconstructed historical root mismatch");
            drop(entries);
            self.stage_root_reference(&mut batch, None, id, first, &tree.nodes)?;
            batch.put_cf(&cf, NEXT_KEY, tree.next_id().to_be_bytes());
            metadata[METADATA_BYTES..].copy_from_slice(&id.to_be_bytes());
            batch.put_cf(&cf, metadata_key(root, METADATA_LEVEL, 0), metadata);
            // Tree staging is dropped here, not retained as an unbounded in-memory cache.
        }
        self.stage_proof_lease(&mut batch, root, now.checked_add(idle_secs).context("Proof lease overflow")?)?;
        if !batch.is_empty() {
            self.inner.db.write_opt(batch, &sync_options())?;
        }
        Ok(SnapshotWithDBArc::new(Arc::clone(&self.inner)))
    }

    /// A scheduled lease protects the root until the expiry sweep consumes its index entry.
    /// Expired markers alone do not block age-queue updates. Call under the tree lock.
    pub(super) fn has_proof_lease(&self, root: Felt) -> Result<bool> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let Some(lease) = self.inner.db.get_cf(&cf, root_key(LEASE, root))? else { return Ok(false) };
        let expires = u64::from_be_bytes(lease.as_slice().try_into()?);
        Ok(self.inner.db.get_cf(&cf, expiry_key(expires, root))?.is_some())
    }

    /// Move the root out of block-age selection and atomically replace its expiry index entry.
    /// Keep a later existing deadline on clock rollback; seconds also coalesce same-second hits.
    fn stage_proof_lease(&self, batch: &mut WriteBatch, root: Felt, expires: u64) -> Result<()> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let key = root_key(LEASE, root);
        if let Some(previous) = self.inner.db.get_cf(&cf, &key)? {
            let previous = u64::from_be_bytes(previous.as_slice().try_into()?);
            if previous >= expires && self.inner.db.get_cf(&cf, expiry_key(previous, root))?.is_some() {
                return Ok(());
            }
            batch.delete_cf(&cf, expiry_key(previous, root));
        }
        let used = self.inner.db.get_cf(&cf, root_key(USED, root))?.context("Missing historical use block")?;
        batch.delete_cf(&cf, age_key(u64::from_be_bytes(used.as_slice().try_into()?), root));
        batch.put_cf(&cf, key, expires.to_be_bytes());
        batch.put_cf(&cf, expiry_key(expires, root), []);
        Ok(())
    }

    /// Return one expired lease to the block-age queue without deleting its nodes. The ordinary
    /// pruning pass must still check the settled cutoff and authenticate the permanent archive.
    /// Caller holds the tree lock, serializing expiry with access, recent execution and repair.
    pub(super) fn expire_proof_lease(&self, now: u64) -> Result<bool> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let candidate = self
            .inner
            .db
            .iterator_cf(&cf, IteratorMode::From(EXPIRY, rocksdb::Direction::Forward))
            .next()
            .transpose()?;
        let Some((key, _)) = candidate.filter(|(key, _)| key.starts_with(EXPIRY)) else { return Ok(false) };
        anyhow::ensure!(key.len() == EXPIRY.len() + 40, "Invalid proof lease expiry index");
        let expires = u64::from_be_bytes(key[EXPIRY.len()..EXPIRY.len() + 8].try_into()?);
        if expires > now {
            return Ok(false);
        }
        let root = Felt::from_bytes_be(&key[EXPIRY.len() + 8..].try_into()?);
        let lease = self.inner.db.get_cf(&cf, root_key(LEASE, root))?.context("Missing proof lease")?;
        anyhow::ensure!(lease == expires.to_be_bytes(), "Proof lease expiry mismatch");
        let used = self.inner.db.get_cf(&cf, root_key(USED, root))?.context("Missing historical use block")?;
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, age_key(u64::from_be_bytes(used.as_slice().try_into()?), root), []);
        // Keep the expired lease marker until actual pruning: a read between expiry and
        // pruning must renew it even when the block buffer still protects the root.
        batch.delete_cf(&cf, key);
        self.inner.db.write_opt(batch, &sync_options())?;
        Ok(true)
    }
}
