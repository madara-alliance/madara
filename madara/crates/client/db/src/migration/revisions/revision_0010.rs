//! Migration from v9 to v10: no-op (new columns are created lazily when opened).

use crate::migration::{MigrationContext, MigrationError};

pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v9→v10 migration: no-op");
    Ok(())
}
