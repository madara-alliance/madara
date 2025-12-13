//! Migration from v8 to v9: SierraClassInfo and StateDiff format changes
//!
//! Updates:
//! - SierraClassInfo: `compiled_class_hash: Felt` → `Option<Felt>`, adds `compiled_class_hash_v2`
//! - StateDiff: adds `migrated_compiled_classes: Vec<MigratedClassItem>`

use crate::migration::{MigrationContext, MigrationError};
use crate::storage::ClassInfoWithBlockN;
use bincode::Options;
use mp_class::{ClassInfo, FlattenedSierraClass, LegacyClassInfo, SierraClassInfo};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
};
use rocksdb::{FlushOptions, IteratorMode, ReadOptions, WriteBatch, WriteOptions};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

const CLASS_INFO_COLUMN: &str = "class_info";
const BLOCK_STATE_DIFF_COLUMN: &str = "block_state_diff";
const BATCH_SIZE: usize = 2000;
const LOG_INTERVAL_SECS: u64 = 10;

fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

// Old format structs for deserialization
#[derive(serde::Deserialize)]
struct SierraClassInfoOld {
    pub contract_class: Arc<FlattenedSierraClass>,
    pub compiled_class_hash: Felt,
}

#[derive(serde::Deserialize)]
#[allow(clippy::enum_variant_names)]
enum ClassInfoOld {
    Sierra(SierraClassInfoOld),
    #[allow(dead_code)]
    Legacy(LegacyClassInfo),
}

#[derive(serde::Deserialize)]
struct ClassInfoWithBlockNOld {
    pub block_number: u64,
    pub class_info: ClassInfoOld,
}

#[derive(serde::Deserialize)]
struct StateDiffOld {
    pub storage_diffs: Vec<ContractStorageDiffItem>,
    pub old_declared_contracts: Vec<Felt>,
    pub declared_classes: Vec<DeclaredClassItem>,
    pub deployed_contracts: Vec<DeployedContractItem>,
    pub replaced_classes: Vec<ReplacedClassItem>,
    pub nonces: Vec<NonceUpdate>,
}

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v8→v9 migration: SierraClassInfo and StateDiff format changes");

    let class_stats = migrate_column(ctx, CLASS_INFO_COLUMN, convert_class_info)?;
    tracing::info!("ClassInfo: {} migrated, {} skipped", class_stats.0, class_stats.1);

    let diff_stats = migrate_column(ctx, BLOCK_STATE_DIFF_COLUMN, convert_state_diff)?;
    tracing::info!("StateDiff: {} migrated, {} skipped", diff_stats.0, diff_stats.1);

    tracing::info!("v8→v9 migration completed");
    Ok(())
}

/// Migrate a column family in batches.
fn migrate_column<F>(ctx: &MigrationContext<'_>, column: &str, convert: F) -> Result<(usize, usize), MigrationError>
where
    F: Fn(&[u8]) -> Option<Vec<u8>>,
{
    tracing::info!("  Migrating column '{}'...", column);

    let db = ctx.db();
    let cf = db.cf_handle(column).ok_or_else(|| MigrationError::RocksDb(format!("{column} not found")))?;

    // Optimize for bulk writes (WAL disabled - backup provides safety)
    let mut write_options = WriteOptions::default();
    write_options.disable_wal(true);
    write_options.set_sync(false);

    let mut read_options = ReadOptions::default();
    read_options.fill_cache(false);

    let start_time = std::time::Instant::now();
    let mut migrated = 0usize;
    let mut skipped = 0usize;
    let mut batch_num = 0usize;
    let mut last_log = start_time;

    let mut batch_buffer: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(BATCH_SIZE);

    for result in db.iterator_cf_opt(&cf, read_options, IteratorMode::Start) {
        let (key, value) = result?;

        if ctx.should_abort() {
            tracing::warn!("  Migration aborted at batch {}", batch_num);
            return Err(MigrationError::Aborted);
        }

        if let Some(new_value) = convert(&value) {
            batch_buffer.push((key.to_vec(), new_value));
            migrated += 1;
        } else {
            skipped += 1;
        }

        // Write batch when full
        if batch_buffer.len() >= BATCH_SIZE {
            batch_num += 1;
            let mut batch = WriteBatch::default();
            for (k, v) in batch_buffer.drain(..) {
                batch.put_cf(&cf, &k, v);
            }
            db.write_opt(batch, &write_options)?;

            // Log first batch completion (helps debug if stuck)
            if batch_num == 1 {
                tracing::info!(
                    "    First batch written ({} entries) in {:.1}s",
                    BATCH_SIZE,
                    start_time.elapsed().as_secs_f64()
                );
            }
        }

        // Log progress periodically
        let now = std::time::Instant::now();
        if now.duration_since(last_log).as_secs() >= LOG_INTERVAL_SECS {
            let elapsed = now.duration_since(start_time).as_secs_f64();
            let total = migrated + skipped;
            let rate = total as f64 / elapsed;
            tracing::info!(
                "    Progress: {} entries ({:.0}/s), migrated: {}, skipped: {}",
                total,
                rate,
                migrated,
                skipped
            );
            last_log = now;
        }
    }

    // Write remaining
    if !batch_buffer.is_empty() {
        batch_num += 1;
        let mut batch = WriteBatch::default();
        for (k, v) in batch_buffer.drain(..) {
            batch.put_cf(&cf, &k, v);
        }
        db.write_opt(batch, &write_options)?;
    }

    // Flush to disk (critical when WAL disabled)
    tracing::info!("    Flushing to disk...");
    let mut flush_opts = FlushOptions::default();
    flush_opts.set_wait(true);
    db.flush_cf_opt(&cf, &flush_opts)
        .map_err(|e| MigrationError::RocksDb(format!("Flush failed for '{column}': {e}")))?;

    let elapsed = start_time.elapsed().as_secs_f64();
    let total = migrated + skipped;
    tracing::info!(
        "  Column '{}' done: {} entries in {} batches, {:.1}s ({:.0}/s)",
        column,
        total,
        batch_num,
        elapsed,
        total as f64 / elapsed.max(0.001)
    );

    Ok((migrated, skipped))
}

/// Convert ClassInfo: old format → new format, returns None if already migrated or legacy
fn convert_class_info(value: &[u8]) -> Option<Vec<u8>> {
    let old: ClassInfoWithBlockNOld = bincode_opts().deserialize(value).ok()?;

    let new_class = match old.class_info {
        ClassInfoOld::Sierra(s) => ClassInfo::Sierra(SierraClassInfo {
            contract_class: s.contract_class,
            compiled_class_hash: Some(s.compiled_class_hash),
            compiled_class_hash_v2: None,
        }),
        ClassInfoOld::Legacy(_) => return None, // Legacy unchanged
    };

    let new_entry = ClassInfoWithBlockN { block_number: old.block_number, class_info: new_class };
    bincode_opts().serialize(&new_entry).ok()
}

/// Convert StateDiff: add migrated_compiled_classes field
fn convert_state_diff(value: &[u8]) -> Option<Vec<u8>> {
    let old: StateDiffOld = bincode_opts().deserialize(value).ok()?;

    let new = StateDiff {
        storage_diffs: old.storage_diffs,
        old_declared_contracts: old.old_declared_contracts,
        declared_classes: old.declared_classes,
        deployed_contracts: old.deployed_contracts,
        replaced_classes: old.replaced_classes,
        nonces: old.nonces,
        migrated_compiled_classes: Vec::new(),
    };

    bincode_opts().serialize(&new).ok()
}
