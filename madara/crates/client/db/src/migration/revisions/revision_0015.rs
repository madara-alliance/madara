//! Migration from v14 to v15: move singleton preconfirmed rows to block-keyed layout.

use crate::migration::{MigrationContext, MigrationError};
use bincode::Options;
use mp_block::header::PreconfirmedHeader;
use rocksdb::WriteBatch;

const META_COLUMN: &str = "meta";
const PRECONFIRMED_COLUMN: &str = "preconfirmed";
const META_HEAD_PROJECTION_KEY: &[u8] = b"HEAD_PROJECTION";
const META_HEAD_PROJECTION_LEGACY_KEY: &[u8] = &[67, 72, 65, 73, 78, 95, 84, 73, 80];
const META_LATEST_APPLIED_TRIE_UPDATE: &[u8] = b"LATEST_APPLIED_TRIE_UPDATE";
const META_PARALLEL_MERKLE_CHECKPOINT_PREFIX: &[u8] = b"PARALLEL_MERKLE_CHECKPOINT/";
const META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY: &[u8] = b"PARALLEL_MERKLE_LATEST_CHECKPOINT";
const META_PRECONFIRMED_HEADER_PREFIX: &[u8] = b"PRECONFIRMED_HEADER/";

#[derive(serde::Deserialize)]
#[allow(dead_code)]
enum StoredHeadProjectionWithoutContent {
    Confirmed(u64),
    Preconfirmed(PreconfirmedHeader),
}

/// Returns the legacy-compatible bincode options used by v14 metadata.
/// Migration reads and writes must use the same encoding as the original rows.
fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

/// Encodes a block number and transaction index into the v15 content key layout.
/// Big-endian fields preserve block and transaction ordering during RocksDB scans.
fn preconfirmed_content_key(block_n: u64, tx_index: u16) -> [u8; 10] {
    let mut key = [0u8; 10];
    key[..8].copy_from_slice(&block_n.to_be_bytes());
    key[8..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

/// Builds the metadata key for one block-keyed preconfirmed header.
/// The fixed prefix separates header records from other metadata rows.
fn preconfirmed_header_key(block_n: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(META_PRECONFIRMED_HEADER_PREFIX.len() + 8);
    key.extend_from_slice(META_PRECONFIRMED_HEADER_PREFIX);
    key.extend_from_slice(&block_n.to_be_bytes());
    key
}

/// Appends a big-endian block number to a metadata namespace prefix.
/// Checkpoint markers and other ordered metadata use this shared layout.
fn meta_key_with_block_n(prefix: &[u8], block_n: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + 8);
    key.extend_from_slice(prefix);
    key.extend_from_slice(&block_n.to_be_bytes());
    key
}

/// Derives the last confirmed block represented by the legacy head projection.
/// A preconfirmed head points one block beyond its confirmed parent.
fn latest_confirmed_from_projection(projection: &StoredHeadProjectionWithoutContent) -> Option<u64> {
    match projection {
        StoredHeadProjectionWithoutContent::Confirmed(block_n) => Some(*block_n),
        StoredHeadProjectionWithoutContent::Preconfirmed(header) => header.block_number.checked_sub(1),
    }
}

/// Loads the current head projection, accepting both the canonical and legacy metadata keys.
/// A missing projection means the database has no preconfirmed state for this migration to move.
fn load_head_projection(
    ctx: &MigrationContext<'_>,
) -> Result<Option<StoredHeadProjectionWithoutContent>, MigrationError> {
    let db = ctx.db();
    let meta_cf = db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb("meta CF missing".to_string()))?;
    let projection_raw = if let Some(raw) = db.get_pinned_cf(&meta_cf, META_HEAD_PROJECTION_KEY)? {
        Some(raw)
    } else {
        db.get_pinned_cf(&meta_cf, META_HEAD_PROJECTION_LEGACY_KEY)?
    };

    projection_raw
        .map(|raw| {
            bincode_opts()
                .deserialize(&raw)
                .map_err(|e| MigrationError::Serialization(format!("deserialize head projection: {e}")))
        })
        .transpose()
}

/// Reads the durable trie progress marker used to seed a parallel-Merkle checkpoint.
/// Missing metadata is valid and falls back to the confirmed portion of the head projection.
fn load_latest_applied_trie_update(ctx: &MigrationContext<'_>) -> Result<Option<u64>, MigrationError> {
    let db = ctx.db();
    let meta_cf = db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb("meta CF missing".to_string()))?;
    db.get_pinned_cf(&meta_cf, META_LATEST_APPLIED_TRIE_UPDATE)?
        .map(|raw| {
            bincode_opts()
                .deserialize::<u64>(&raw)
                .map_err(|e| MigrationError::Serialization(format!("deserialize latest applied trie update: {e}")))
        })
        .transpose()
}

/// Adds the checkpoint marker and latest-checkpoint pointer to the migration write batch.
/// Both records are staged together so an interrupted migration cannot expose only one of them.
fn stage_checkpoint(
    ctx: &MigrationContext<'_>,
    batch: &mut WriteBatch,
    checkpoint_block_n: u64,
) -> Result<(), MigrationError> {
    let db = ctx.db();
    let meta_cf = db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb("meta CF missing".to_string()))?;
    batch.put_cf(&meta_cf, meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, checkpoint_block_n), [1u8]);
    batch.put_cf(
        &meta_cf,
        META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY,
        bincode_opts()
            .serialize(&checkpoint_block_n)
            .map_err(|e| MigrationError::Serialization(format!("serialize latest checkpoint: {e}")))?,
    );
    Ok(())
}

/// Moves legacy two-byte transaction keys under the projected preconfirmed block number.
/// The corresponding block-keyed header is staged in the same batch and the moved-row count is returned.
fn stage_preconfirmed_rows(
    ctx: &MigrationContext<'_>,
    batch: &mut WriteBatch,
    projection: StoredHeadProjectionWithoutContent,
) -> Result<usize, MigrationError> {
    let StoredHeadProjectionWithoutContent::Preconfirmed(header) = projection else {
        tracing::info!("v14→v15 migration: head is not preconfirmed, skipping legacy preconfirmed row migration");
        return Ok(0);
    };

    let db = ctx.db();
    let meta_cf = db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb("meta CF missing".to_string()))?;
    let preconfirmed_cf = db
        .cf_handle(PRECONFIRMED_COLUMN)
        .ok_or_else(|| MigrationError::RocksDb("preconfirmed CF missing".to_string()))?;
    let block_n = header.block_number;
    let mut moved = 0;

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
    Ok(moved)
}

/// Migrates v14 head metadata and singleton preconfirmed rows into the v15 keyed layout.
/// All mutations share one write batch, so rerunning an interrupted migration is idempotent.
pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v14→v15 migration: block-keyed preconfirmed persistence");

    let Some(projection) = load_head_projection(ctx)? else {
        tracing::info!("v14→v15 migration: no head projection found, nothing to migrate");
        return Ok(());
    };
    let latest_applied_trie_update = load_latest_applied_trie_update(ctx)?;
    let checkpoint_seed = latest_applied_trie_update.or_else(|| latest_confirmed_from_projection(&projection));

    let mut batch = WriteBatch::default();
    if let Some(checkpoint_block_n) = checkpoint_seed {
        stage_checkpoint(ctx, &mut batch, checkpoint_block_n)?;
    }
    let moved = stage_preconfirmed_rows(ctx, &mut batch, projection)?;
    ctx.db().write(batch)?;

    tracing::info!(
        "v14→v15 migration completed: moved {moved} legacy preconfirmed rows, checkpoint_seed={checkpoint_seed:?}, latest_applied_trie_update={latest_applied_trie_update:?}"
    );
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

    fn latest_checkpoint(db: &rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>) -> Option<u64> {
        let meta_cf = db.cf_handle(META_COLUMN).expect("meta cf");
        db.get_pinned_cf(&meta_cf, META_PARALLEL_MERKLE_LATEST_CHECKPOINT_KEY)
            .expect("read latest checkpoint")
            .map(|raw| bincode_opts().deserialize(&raw).expect("decode latest checkpoint"))
    }

    fn has_checkpoint(db: &rocksdb::DBWithThreadMode<rocksdb::MultiThreaded>, block_n: u64) -> bool {
        let meta_cf = db.cf_handle(META_COLUMN).expect("meta cf");
        db.get_pinned_cf(&meta_cf, meta_key_with_block_n(META_PARALLEL_MERKLE_CHECKPOINT_PREFIX, block_n))
            .expect("read checkpoint marker")
            .is_some()
    }

    #[rstest]
    #[case::legacy_only(true, false)]
    #[case::legacy_and_block_keyed(true, true)]
    fn migration_v15_is_idempotent(#[case] with_legacy_rows: bool, #[case] with_block_keyed_rows: bool) {
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

        let ctx = MigrationContext::new(&db, tmp.path(), Arc::new(AtomicBool::new(false)));
        migrate(&ctx).expect("first migration");
        migrate(&ctx).expect("second migration (idempotent)");

        let header_key = preconfirmed_header_key(block_n);
        let stored_header = db.get_pinned_cf(&meta_cf, header_key).expect("read header").expect("header present");
        let decoded_header: PreconfirmedHeader = bincode_opts().deserialize(&stored_header).expect("decode header");
        assert_eq!(decoded_header.block_number, block_n);
        assert_eq!(latest_checkpoint(&db), Some(block_n - 1));
        assert!(has_checkpoint(&db, block_n - 1));

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

    #[test]
    fn migration_v15_seeds_checkpoint_for_confirmed_head() {
        let tmp = TempDir::new().expect("tempdir");
        let db = open_db(&tmp);
        let meta_cf = db.cf_handle(META_COLUMN).expect("meta cf");

        let block_n = 11u64;
        let projection = StoredHeadProjectionWithoutContentTest::Confirmed(block_n);
        db.put_cf(
            &meta_cf,
            META_HEAD_PROJECTION_KEY,
            bincode_opts().serialize(&projection).expect("serialize projection"),
        )
        .expect("write projection");

        let ctx = MigrationContext::new(&db, tmp.path(), Arc::new(AtomicBool::new(false)));
        migrate(&ctx).expect("migration");

        assert_eq!(latest_checkpoint(&db), Some(block_n));
        assert!(has_checkpoint(&db, block_n));
    }

    #[test]
    fn migration_v15_prefers_latest_applied_trie_update_for_checkpoint_seed() {
        let tmp = TempDir::new().expect("tempdir");
        let db = open_db(&tmp);
        let meta_cf = db.cf_handle(META_COLUMN).expect("meta cf");

        let header = PreconfirmedHeader { block_number: 9, ..Default::default() };
        let projection = StoredHeadProjectionWithoutContentTest::Preconfirmed(header);
        db.put_cf(
            &meta_cf,
            META_HEAD_PROJECTION_KEY,
            bincode_opts().serialize(&projection).expect("serialize projection"),
        )
        .expect("write projection");
        db.put_cf(
            &meta_cf,
            META_LATEST_APPLIED_TRIE_UPDATE,
            bincode_opts().serialize(&6u64).expect("serialize latest applied trie update"),
        )
        .expect("write latest applied trie update");

        let ctx = MigrationContext::new(&db, tmp.path(), Arc::new(AtomicBool::new(false)));
        migrate(&ctx).expect("migration");

        assert_eq!(latest_checkpoint(&db), Some(6));
        assert!(has_checkpoint(&db, 6));
    }
}
