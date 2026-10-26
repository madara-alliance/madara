//! v16 → v17 introduces immutable shared committed-data nodes and root pointers.
//! Legacy paged roots remain readable and are converted only by a later full import/update.
//! Bumping the schema prevents older binaries from reading a root-pointer record as a snapshot.
use crate::migration::{MigrationContext, MigrationError};

pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("v16→v17: shared committed-data nodes; legacy roots preserved without rewrite");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::migration::{MigrationRunner, MigrationStatus};
    use rocksdb::{DBWithThreadMode, MultiThreaded, Options};

    #[test]
    fn committed_data_v17_upgrade_preserves_meta_records_across_reopen_and_rejects_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let data_path = dir.path().join("db");
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let rows: Vec<(&[u8], &[u8])> = vec![
            (b"committed_data_mpt/test", b"immutable dataset page"),
            (b"HEAD_PROJECTION", b"unrelated chain metadata"),
        ];
        {
            let db = DBWithThreadMode::<MultiThreaded>::open_cf(&options, &data_path, ["meta"]).unwrap();
            let cf = db.cf_handle("meta").unwrap();
            for (key, value) in &rows {
                db.put_cf(&cf, key, value).unwrap();
            }
            let runner = MigrationRunner::new(dir.path(), 17, 8);
            runner.write_version_file(16).unwrap();
            runner.run_migrations(&db).unwrap();
            assert_eq!(runner.read_version_file().unwrap(), 17);
            runner.run_migrations(&db).unwrap(); // Already migrated: idempotent startup.
        }
        let db = DBWithThreadMode::<MultiThreaded>::open_cf(&options, &data_path, ["meta"]).unwrap();
        let cf = db.cf_handle("meta").unwrap();
        for (key, value) in rows {
            assert_eq!(db.get_cf(&cf, key).unwrap().as_deref(), Some(value));
        }
        assert!(matches!(
            MigrationRunner::new(dir.path(), 16, 8).check_status().unwrap(),
            MigrationStatus::DatabaseNewer { db_version: 17, binary_version: 16 }
        ));
    }
}
