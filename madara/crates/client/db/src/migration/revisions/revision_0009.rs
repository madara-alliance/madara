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
const BATCH_SIZE: usize = 2000; // Increased from 500 to reduce write overhead

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
    tracing::info!("Starting migration revision 0009: SierraClassInfo and StateDiff format changes");
    
    let class_stats = migrate_column(ctx, CLASS_INFO_COLUMN, convert_class_info)?;
    tracing::info!("ClassInfo migration completed: {} migrated, {} skipped", class_stats.0, class_stats.1);

    let diff_stats = migrate_column(ctx, BLOCK_STATE_DIFF_COLUMN, convert_state_diff)?;
    tracing::info!("StateDiff migration completed: {} migrated, {} skipped", diff_stats.0, diff_stats.1);

    tracing::info!("Migration revision 0009 completed successfully");
    Ok(())
}

/// Generic column migration: returns (migrated_count, skipped_count)
/// Processes entries in a streaming fashion to avoid loading everything into memory.
fn migrate_column<F>(ctx: &MigrationContext<'_>, column: &str, convert: F) -> Result<(usize, usize), MigrationError>
where
    F: Fn(&[u8]) -> Option<Vec<u8>>,
{
    let db = ctx.db();
    let cf = db.cf_handle(column).ok_or_else(|| MigrationError::RocksDb(format!("{column} not found")))?;

    tracing::info!("Starting migration for column '{}'", column);
    
    // Optimize WriteOptions: disable WAL and sync for faster writes (safe since we have backups)
    let mut write_options = WriteOptions::default();
    write_options.disable_wal(true);
    write_options.set_sync(false);
    
    // Optimize ReadOptions: don't fill cache during migration to save memory
    let mut read_options = ReadOptions::default();
    read_options.fill_cache(false);
    
    let start_time = std::time::Instant::now();
    let mut migrated = 0;
    let mut skipped = 0;
    let mut batch_num = 0;
    let mut processed_count = 0u64;
    let mut last_log_time = start_time;
    let mut conversion_time = std::time::Duration::ZERO;
    let mut write_time = std::time::Duration::ZERO;
    let mut conversion_count = 0u64;
    
    // Buffer to accumulate entries before writing a batch
    let mut batch_buffer: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(BATCH_SIZE);
    
    let iterator = db
        .iterator_cf_opt(&cf, read_options, IteratorMode::Start)
        .filter_map(|r| r.ok());
    
    for (key, value) in iterator {
        if ctx.should_abort() {
            tracing::warn!("Migration aborted for column '{}' at batch {}", column, batch_num);
            return Err(MigrationError::Aborted);
        }
        
        processed_count += 1;
        
        // Convert the entry (measure conversion time)
        let convert_start = std::time::Instant::now();
        if let Some(new_value) = convert(&value) {
            conversion_time += convert_start.elapsed();
            conversion_count += 1;
            batch_buffer.push((key.to_vec(), new_value));
            migrated += 1;
        } else {
            skipped += 1;
        }
        
        // Log progress every 5 seconds during processing
        let now = std::time::Instant::now();
        if now.duration_since(last_log_time).as_secs() >= 5 {
            let elapsed = now.duration_since(start_time);
            let rate = if elapsed.as_secs() > 0 {
                processed_count as f64 / elapsed.as_secs() as f64
            } else {
                0.0
            };
            let avg_convert_time = if conversion_count > 0 {
                conversion_time.as_millis() as f64 / conversion_count as f64
            } else {
                0.0
            };
            tracing::info!(
                "Column '{}': processed {} entries ({} entries/s), migrated: {}, skipped: {}, avg conversion: {:.2}ms",
                column,
                processed_count,
                rate as u64,
                migrated,
                skipped,
                avg_convert_time
            );
            last_log_time = now;
        }
        
        // Write batch when buffer is full
        if batch_buffer.len() >= BATCH_SIZE {
            batch_num += 1;
            let write_start = std::time::Instant::now();
            
            if batch_num == 1 {
                tracing::info!("Writing first batch for column '{}'...", column);
            }
            
            let mut batch = WriteBatch::default();
            for (k, v) in batch_buffer.drain(..) {
                batch.put_cf(&cf, &k, v);
            }
            
            db.write_opt(batch, &write_options)?;
            let batch_write_duration = write_start.elapsed();
            write_time += batch_write_duration;
            
            // Log per-batch write duration for diagnostics
            if batch_num == 1 || batch_num % 10 == 0 {
                tracing::debug!(
                    "Column '{}': batch {} write took {:.2}ms",
                    column,
                    batch_num,
                    batch_write_duration.as_millis()
                );
            }
            
            if batch_num == 1 {
                let first_batch_duration = start_time.elapsed();
                tracing::info!(
                    "First batch for column '{}' completed in {:.2}s",
                    column,
                    first_batch_duration.as_secs_f64()
                );
            }
            
            // Log progress every 10 batches with timing breakdown
            if batch_num % 10 == 0 {
                let elapsed = start_time.elapsed();
                let total_time_ms = elapsed.as_millis() as f64;
                let conversion_pct = if total_time_ms > 0.0 {
                    (conversion_time.as_millis() as f64 / total_time_ms) * 100.0
                } else {
                    0.0
                };
                let write_pct = if total_time_ms > 0.0 {
                    (write_time.as_millis() as f64 / total_time_ms) * 100.0
                } else {
                    0.0
                };
                tracing::info!(
                    "Column '{}': batch {}, processed: {}, migrated: {}, skipped: {} | Time breakdown: conversion {:.1}%, writes {:.1}%",
                    column,
                    batch_num,
                    processed_count,
                    migrated,
                    skipped,
                    conversion_pct,
                    write_pct
                );
            }
        }
    }
    
    // Write any remaining entries in the buffer
    if !batch_buffer.is_empty() {
        batch_num += 1;
        let mut batch = WriteBatch::default();
        for (k, v) in batch_buffer.drain(..) {
            batch.put_cf(&cf, &k, v);
        }
        db.write_opt(batch, &write_options)?;
    }
    
    // Flush the column family to ensure all data is persisted (critical when WAL is disabled)
    let mut flush_opts = FlushOptions::default();
    flush_opts.set_wait(true);
    db.flush_cf_opt(&cf, &flush_opts)
        .map_err(|e| MigrationError::RocksDb(format!("Failed to flush column '{}': {}", column, e)))?;
    tracing::info!("Flushed column '{}' to disk", column);
    
    let total_duration = start_time.elapsed();
    let total_time_ms = total_duration.as_millis() as f64;
    let conversion_pct = if total_time_ms > 0.0 {
        (conversion_time.as_millis() as f64 / total_time_ms) * 100.0
    } else {
        0.0
    };
    let write_pct = if total_time_ms > 0.0 {
        (write_time.as_millis() as f64 / total_time_ms) * 100.0
    } else {
        0.0
    };
    let other_pct = 100.0 - conversion_pct - write_pct;
    let avg_rate = if total_duration.as_secs() > 0 {
        processed_count as f64 / total_duration.as_secs() as f64
    } else {
        0.0
    };
    
    tracing::info!(
        "Column '{}' migration completed: {} batches, {} migrated, {} skipped",
        column,
        batch_num,
        migrated,
        skipped
    );
    tracing::info!(
        "Column '{}' timing: total {:.2}s, conversion {:.1}% ({:.2}s), writes {:.1}% ({:.2}s), other {:.1}% ({:.2}s)",
        column,
        total_duration.as_secs_f64(),
        conversion_pct,
        conversion_time.as_secs_f64(),
        write_pct,
        write_time.as_secs_f64(),
        other_pct,
        (total_duration - conversion_time - write_time).as_secs_f64()
    );
    tracing::info!(
        "Column '{}' performance: {:.0} entries/s average",
        column,
        avg_rate
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
