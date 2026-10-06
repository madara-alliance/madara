//! v15 → v16 records additive, root-keyed committed-data storage in the existing meta column.
//! No chain data or dataset pages are rewritten. Databases without imported data remain empty
//! in this namespace. Existing v2 pages and the shared quota counter are preserved. Legacy
//! publisher-bound roots cannot be converted: rebuild and explicitly import their datasets.
use crate::migration::{MigrationContext, MigrationError};

/// Records the schema boundary through the migration runner, without modifying any DB records.
pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("v15→v16: additive committed-data records; no data rewrite required");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::migration::{MigrationRunner, MigrationStatus};
    use rocksdb::{DBWithThreadMode, MultiThreaded, Options};

    #[test]
    fn committed_data_v16_upgrade_preserves_meta_records_across_reopen_and_rejects_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let data_path = dir.path().join("db");
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let rows: Vec<(&[u8], &[u8])> = vec![
            (b"committed_data_pages_v2/test", b"immutable dataset page"),
            (b"committed_data_pages_v1/test", b"legacy publisher-bound page"),
            (b"committed_data_pages_usage_v1", b"quota counter"),
            (b"HEAD_PROJECTION", b"unrelated chain metadata"),
        ];
        {
            let db = DBWithThreadMode::<MultiThreaded>::open_cf(&options, &data_path, ["meta"]).unwrap();
            let cf = db.cf_handle("meta").unwrap();
            for (key, value) in &rows {
                db.put_cf(&cf, key, value).unwrap();
            }
            let runner = MigrationRunner::new(dir.path(), 16, 8);
            runner.write_version_file(15).unwrap();
            runner.run_migrations(&db).unwrap();
            assert_eq!(runner.read_version_file().unwrap(), 16);
            runner.run_migrations(&db).unwrap(); // Already migrated: idempotent startup.
        }
        let db = DBWithThreadMode::<MultiThreaded>::open_cf(&options, &data_path, ["meta"]).unwrap();
        let cf = db.cf_handle("meta").unwrap();
        for (key, value) in rows {
            assert_eq!(db.get_cf(&cf, key).unwrap().as_deref(), Some(value));
        }
        assert!(matches!(
            MigrationRunner::new(dir.path(), 15, 8).check_status().unwrap(),
            MigrationStatus::DatabaseNewer { db_version: 16, binary_version: 15 }
        ));
    }
}
