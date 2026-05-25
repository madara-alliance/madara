//! Migration from v13 to v14: seed the external DB retention cursor.

use crate::migration::{MigrationContext, MigrationError, MigrationProgress};
use rocksdb::FlushOptions;

const META_COLUMN: &str = "meta";
const META_CONFIRMED_ON_L1_TIP_KEY: &[u8] = b"CONFIRMED_ON_L1_TIP";
const META_EXTERNAL_DB_RETENTION_CURSOR_KEY: &[u8] = b"EXTERNAL_DB_RETENTION_CURSOR";

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const RETENTION_DAYS: u64 = 3;
const ASSUMED_BLOCK_TIME_SECONDS: u64 = 15;
const RETENTION_BLOCKS: u64 = (SECONDS_PER_DAY * RETENTION_DAYS) / ASSUMED_BLOCK_TIME_SECONDS;

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v13→v14 migration: seeding external DB retention cursor");
    ctx.report_progress(MigrationProgress::new(0, 3, "Inspecting retention cursor state"));

    let db = ctx.db();
    let meta_cf =
        db.cf_handle(META_COLUMN).ok_or_else(|| MigrationError::RocksDb(format!("{META_COLUMN} not found")))?;

    if ctx.should_abort() {
        return Err(MigrationError::Aborted);
    }

    if db.get_pinned_cf(&meta_cf, META_EXTERNAL_DB_RETENTION_CURSOR_KEY)?.is_some() {
        tracing::info!(
            target: "madara_db_migration",
            "external_db_retention_cursor_already_present"
        );
        ctx.report_progress(MigrationProgress::new(3, 3, "Retention cursor already present"));
        return Ok(());
    }

    ctx.report_progress(MigrationProgress::new(1, 3, "Loading latest L1-confirmed block"));
    let Some(latest_confirmed_on_l1) = read_optional_u64(db.get_pinned_cf(&meta_cf, META_CONFIRMED_ON_L1_TIP_KEY)?)?
    else {
        tracing::info!(
            target: "madara_db_migration",
            "external_db_retention_cursor_not_seeded_no_l1_tip"
        );
        ctx.report_progress(MigrationProgress::new(3, 3, "No L1-confirmed block recorded yet"));
        return Ok(());
    };

    if ctx.should_abort() {
        return Err(MigrationError::Aborted);
    }

    let seeded_cursor = latest_confirmed_on_l1.saturating_sub(RETENTION_BLOCKS);
    db.put_cf(&meta_cf, META_EXTERNAL_DB_RETENTION_CURSOR_KEY, seeded_cursor.to_be_bytes())?;

    let mut flush_opts = FlushOptions::default();
    flush_opts.set_wait(true);
    db.flush_cf_opt(&meta_cf, &flush_opts)?;

    tracing::info!(
        target: "madara_db_migration",
        latest_confirmed_on_l1,
        seeded_cursor,
        retention_blocks = RETENTION_BLOCKS,
        "seeded_external_db_retention_cursor"
    );
    ctx.report_progress(MigrationProgress::new(3, 3, "Seeded external DB retention cursor"));
    Ok(())
}

fn read_optional_u64(value: Option<impl AsRef<[u8]>>) -> Result<Option<u64>, MigrationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let bytes = value.as_ref();
    let array: [u8; 8] =
        bytes.try_into().map_err(|_| MigrationError::Serialization("Malformed u64 metadata value".to_string()))?;
    Ok(Some(u64::from_be_bytes(array)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_cursor_uses_three_day_fifteen_second_window() {
        assert_eq!(RETENTION_BLOCKS, 17_280);
        assert_eq!(20_000u64.saturating_sub(RETENTION_BLOCKS), 2_720);
        assert_eq!(100u64.saturating_sub(RETENTION_BLOCKS), 0);
    }
}
