//! Migration from v10 to v11: no-op (new columns are created lazily when opened).

use crate::migration::{MigrationContext, MigrationError};

pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v10→v11 migration: no-op");
    Ok(())
}
