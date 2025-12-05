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
use rocksdb::{IteratorMode, ReadOptions, WriteBatch};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

const CLASS_INFO_COLUMN: &str = "class_info";
const BLOCK_STATE_DIFF_COLUMN: &str = "block_state_diff";
const BATCH_SIZE: usize = 500;

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
    let class_stats = migrate_column(ctx, CLASS_INFO_COLUMN, convert_class_info)?;
    tracing::info!("ClassInfo: {} migrated, {} skipped", class_stats.0, class_stats.1);

    let diff_stats = migrate_column(ctx, BLOCK_STATE_DIFF_COLUMN, convert_state_diff)?;
    tracing::info!("StateDiff: {} migrated, {} skipped", diff_stats.0, diff_stats.1);

    Ok(())
}

/// Generic column migration: returns (migrated_count, skipped_count)
fn migrate_column<F>(ctx: &MigrationContext<'_>, column: &str, convert: F) -> Result<(usize, usize), MigrationError>
where
    F: Fn(&[u8]) -> Option<Vec<u8>>,
{
    let db = ctx.db();
    let cf = db.cf_handle(column).ok_or_else(|| MigrationError::RocksDb(format!("{column} not found")))?;

    let entries: Vec<_> = db
        .iterator_cf_opt(&cf, ReadOptions::default(), IteratorMode::Start)
        .filter_map(|r| r.ok())
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect();

    let (mut migrated, mut skipped) = (0, 0);

    for chunk in entries.chunks(BATCH_SIZE) {
        if ctx.should_abort() {
            return Err(MigrationError::Aborted);
        }

        let mut batch = WriteBatch::default();
        for (key, value) in chunk {
            if let Some(new_value) = convert(value) {
                batch.put_cf(&cf, key, new_value);
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        db.write(batch)?;
    }

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
