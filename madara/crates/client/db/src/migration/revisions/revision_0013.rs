//! Migration from v12 to v13: move singleton preconfirmed rows to block-keyed layout.

use crate::migration::{MigrationContext, MigrationError};
use bincode::Options;
use mp_block::header::PreconfirmedHeader;
use rocksdb::WriteBatch;

const META_COLUMN: &str = "meta";
const PRECONFIRMED_COLUMN: &str = "preconfirmed";
const META_HEAD_PROJECTION_KEY: &[u8] = b"HEAD_PROJECTION";
const META_HEAD_PROJECTION_LEGACY_KEY: &[u8] = &[67, 72, 65, 73, 78, 95, 84, 73, 80];
const META_PRECONFIRMED_HEADER_PREFIX: &[u8] = b"PRECONFIRMED_HEADER/";

#[derive(serde::Deserialize)]
#[allow(dead_code)]
enum StoredHeadProjectionWithoutContent {
    Confirmed(u64),
    Preconfirmed(PreconfirmedHeader),
}

fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

fn preconfirmed_content_key(block_n: u64, tx_index: u16) -> [u8; 10] {
    let mut key = [0u8; 10];
    key[..8].copy_from_slice(&block_n.to_be_bytes());
    key[8..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

fn preconfirmed_header_key(block_n: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(META_PRECONFIRMED_HEADER_PREFIX.len() + 8);
    key.extend_from_slice(META_PRECONFIRMED_HEADER_PREFIX);
    key.extend_from_slice(&block_n.to_be_bytes());
    key
}

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v12→v13 migration: block-keyed preconfirmed persistence");

    let db = ctx.db();
    let meta_cf = db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb("meta CF missing".to_string()))?;
    let preconfirmed_cf = db
        .cf_handle(PRECONFIRMED_COLUMN)
        .ok_or_else(|| MigrationError::RocksDb("preconfirmed CF missing".to_string()))?;

    let projection_raw = if let Some(raw) = db.get_pinned_cf(&meta_cf, META_HEAD_PROJECTION_KEY)? {
        Some(raw)
    } else {
        db.get_pinned_cf(&meta_cf, META_HEAD_PROJECTION_LEGACY_KEY)?
    };

    let Some(projection_raw) = projection_raw else {
        tracing::info!("v12→v13 migration: no head projection found, nothing to migrate");
        return Ok(());
    };

    let projection: StoredHeadProjectionWithoutContent = bincode_opts()
        .deserialize(&projection_raw)
        .map_err(|e| MigrationError::Serialization(format!("deserialize head projection: {e}")))?;

    let StoredHeadProjectionWithoutContent::Preconfirmed(header) = projection else {
        tracing::info!("v12→v13 migration: head is not preconfirmed, nothing to migrate");
        return Ok(());
    };

    let block_n = header.block_number;
    let mut batch = WriteBatch::default();
    let mut moved = 0usize;

    for item in db.iterator_cf(&preconfirmed_cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item?;
        if key.len() != 2 {
            continue;
        }
        let tx_index = u16::from_be_bytes(
            key.as_ref()
                .try_into()
                .map_err(|_| MigrationError::Serialization("malformed legacy preconfirmed key".to_string()))?,
        );
        batch.put_cf(&preconfirmed_cf, preconfirmed_content_key(block_n, tx_index), value);
        batch.delete_cf(&preconfirmed_cf, key);
        moved += 1;
    }

    batch.put_cf(
        &meta_cf,
        preconfirmed_header_key(block_n),
        bincode_opts()
            .serialize(&header)
            .map_err(|e| MigrationError::Serialization(format!("serialize preconfirmed header: {e}")))?,
    );
    db.write(batch)?;

    tracing::info!("v12→v13 migration completed: moved {moved} legacy preconfirmed rows for block #{block_n}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::MigrationContext;
    use bincode::Options as _;
    use rocksdb::{ColumnFamilyDescriptor, Options};
    use rstest::rstest;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[derive(serde::Serialize)]
    #[allow(dead_code)]
    enum StoredHeadProjectionWithoutContentTest {
        Confirmed(u64),
        Preconfirmed(PreconfirmedHeader),
    }

    fn open_db(tmp: &TempDir) -> rocksdb::DBWithThreadMode<rocksdb::MultiThreaded> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        rocksdb::DBWithThreadMode::<rocksdb::MultiThreaded>::open_cf_descriptors(
            &opts,
            tmp.path(),
            [
                ColumnFamilyDescriptor::new(META_COLUMN, Options::default()),
                ColumnFamilyDescriptor::new(PRECONFIRMED_COLUMN, Options::default()),
            ],
        )
        .expect("open db")
    }

    #[rstest]
    #[case::legacy_only(true, false)]
    #[case::legacy_and_block_keyed(true, true)]
    fn migration_v13_is_idempotent(#[case] with_legacy_rows: bool, #[case] with_block_keyed_rows: bool) {
        let tmp = TempDir::new().expect("tempdir");
        let db = open_db(&tmp);
        let meta_cf = db.cf_handle(META_COLUMN).expect("meta cf");
        let preconfirmed_cf = db.cf_handle(PRECONFIRMED_COLUMN).expect("preconfirmed cf");

        let block_n = 7u64;
        let header = PreconfirmedHeader { block_number: block_n, ..Default::default() };
        let projection = StoredHeadProjectionWithoutContentTest::Preconfirmed(header.clone());
        db.put_cf(
            &meta_cf,
            META_HEAD_PROJECTION_KEY,
            bincode_opts().serialize(&projection).expect("serialize projection"),
        )
        .expect("write projection");

        if with_legacy_rows {
            db.put_cf(&preconfirmed_cf, 0u16.to_be_bytes(), b"tx0").expect("write legacy tx0");
            db.put_cf(&preconfirmed_cf, 1u16.to_be_bytes(), b"tx1").expect("write legacy tx1");
        }
        if with_block_keyed_rows {
            db.put_cf(&preconfirmed_cf, preconfirmed_content_key(block_n, 2), b"tx2").expect("write keyed tx2");
        }

        let ctx = MigrationContext::new(&db, Arc::new(AtomicBool::new(false)));
        migrate(&ctx).expect("first migration");
        migrate(&ctx).expect("second migration (idempotent)");

        let header_key = preconfirmed_header_key(block_n);
        let stored_header = db.get_pinned_cf(&meta_cf, header_key).expect("read header").expect("header present");
        let decoded_header: PreconfirmedHeader = bincode_opts().deserialize(&stored_header).expect("decode header");
        assert_eq!(decoded_header.block_number, block_n);

        assert!(db.get_pinned_cf(&preconfirmed_cf, 0u16.to_be_bytes()).expect("read legacy 0").is_none());
        assert!(db.get_pinned_cf(&preconfirmed_cf, 1u16.to_be_bytes()).expect("read legacy 1").is_none());
        if with_legacy_rows {
            assert_eq!(
                db.get_pinned_cf(&preconfirmed_cf, preconfirmed_content_key(block_n, 0))
                    .expect("read keyed 0")
                    .as_deref(),
                Some(&b"tx0"[..])
            );
            assert_eq!(
                db.get_pinned_cf(&preconfirmed_cf, preconfirmed_content_key(block_n, 1))
                    .expect("read keyed 1")
                    .as_deref(),
                Some(&b"tx1"[..])
            );
        }
        if with_block_keyed_rows {
            assert_eq!(
                db.get_pinned_cf(&preconfirmed_cf, preconfirmed_content_key(block_n, 2))
                    .expect("read keyed 2")
                    .as_deref(),
                Some(&b"tx2"[..])
            );
        }
    }
}
