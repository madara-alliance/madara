//! Split permanent recovery identities from temporary upload payloads.
use crate::{
    committed_data_recovery::RecoveryRecord,
    migration::{MigrationContext, MigrationError},
    rocksdb::{deserialize, serialize},
};
use rocksdb::{Direction, IteratorMode, WriteBatch, WriteOptions};

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    let db = ctx.db();
    let cf = db
        .cf_handle("committed_data_recovery")
        .ok_or_else(|| MigrationError::RocksDb("committed_data_recovery not found".into()))?;
    let mut options = WriteOptions::default();
    options.set_sync(true);
    options.disable_wal(false);
    // One record per atomic batch bounds memory and makes interrupted upgrades repeatable.
    for item in db.iterator_cf(&cf, IteratorMode::From(b"r", Direction::Forward)) {
        if ctx.should_abort() {
            return Err(MigrationError::Aborted);
        }
        let (key, value) = item?;
        if !key.starts_with(b"r") {
            break;
        }
        let record: RecoveryRecord = deserialize(&value).map_err(|e| MigrationError::Serialization(e.to_string()))?;
        record.manifest.validate().map_err(|e| MigrationError::Serialization(e.to_string()))?;
        let id = record.manifest.id().map_err(|e| MigrationError::Serialization(e.to_string()))?;
        if key[1..] != *id.as_bytes() {
            return Err(MigrationError::Serialization("Recovery identity mismatch".into()));
        }
        let manifest = serialize(&record.manifest).map_err(|e| MigrationError::Serialization(e.to_string()))?;
        let manifest_key = [b"m".as_slice(), id.as_bytes()].concat();
        if let Some(existing) = db.get_cf(&cf, &manifest_key)? {
            if existing != manifest {
                return Err(MigrationError::Serialization("Conflicting recovery manifest".into()));
            }
        }
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, manifest_key, manifest);
        // In v18, removal of p<id> is the durable acknowledgement of Mongo completion.
        // Never discard a record that is still pending or alter its byte accounting.
        if db.get_cf(&cf, [b"p".as_slice(), id.as_bytes()].concat())?.is_none() {
            batch.delete_cf(&cf, key);
        }
        db.write_opt(batch, &options)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
    use mp_convert::Felt;
    use std::sync::{atomic::AtomicBool, Arc};

    #[test]
    fn recovery_migration_preserves_pending_and_removes_acknowledged_payloads() {
        let backend = MadaraBackend::open_for_testing(mp_chain_config::ChainConfig::madara_test().into());
        let db = backend.db.inner_db();
        let cf = db.cf_handle("committed_data_recovery").unwrap();
        let mut ids = Vec::new();
        for root in [Felt::ONE, Felt::TWO] {
            let record = RecoveryRecord::encode(Felt::ONE, None, root, None, vec![(Felt::ONE, root)]).unwrap();
            let id = backend.db.store_recovery_record(&record).unwrap();
            db.delete_cf(&cf, [b"m".as_slice(), id.as_bytes()].concat()).unwrap();
            ids.push(id);
        }
        // Reproduce the old exporter: removes only the pending marker, keeps the payload.
        let pending_key = [b"p".as_slice(), ids[0].as_bytes()].concat();
        let size: u64 = deserialize(db.get_cf(&cf, &pending_key).unwrap().unwrap()).unwrap();
        let remaining = backend.db.recovery_pending_bytes().unwrap() - size;
        db.delete_cf(&cf, pending_key).unwrap();
        db.put_cf(&cf, b"bytes", serialize(&remaining).unwrap()).unwrap();
        let ctx = MigrationContext::new(&db, std::path::Path::new("."), Arc::new(AtomicBool::new(false)));
        let version_dir = tempfile::tempdir().unwrap();
        std::fs::write(version_dir.path().join(crate::migration::DB_VERSION_FILE), "18").unwrap();
        crate::migration::MigrationRunner::new(version_dir.path(), 19, 8).run_migrations(db).unwrap();
        assert_eq!(
            std::fs::read_to_string(version_dir.path().join(crate::migration::DB_VERSION_FILE)).unwrap().trim(),
            "19"
        );
        assert!(matches!(
            crate::migration::MigrationRunner::new(version_dir.path(), 18, 8).run_migrations(db),
            Err(MigrationError::DatabaseNewerThanBinary { .. })
        ));
        assert!(!version_dir.path().join("backup_pre_migration").exists());
        migrate(&ctx).unwrap();
        assert!(backend.db.get_recovery_record(&ids[0]).unwrap().is_none());
        assert!(backend.db.get_recovery_record(&ids[1]).unwrap().is_some());
        for id in &ids {
            assert_eq!(backend.db.get_recovery_manifest(id).unwrap().unwrap().id().unwrap(), *id);
        }
        assert_eq!(backend.db.pending_recovery_records(10).unwrap(), vec![ids[1].clone()]);
        assert_eq!(backend.db.recovery_pending_bytes().unwrap(), remaining);
        db.put_cf(&cf, b"rcorrupt", b"invalid record").unwrap();
        assert!(migrate(&ctx).is_err());
        assert!(backend.db.get_recovery_record(&ids[1]).unwrap().is_some());
    }
}
