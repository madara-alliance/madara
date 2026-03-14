//! Migration from v11 to v12: no-op.

use crate::migration::{MigrationContext, MigrationError};

pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v11→v12 migration: no-op");
    Ok(())
}
