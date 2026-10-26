//! The recovery CF is created by RocksDB column registration; legacy records remain unchanged.
use crate::migration::{MigrationContext, MigrationError};
pub fn migrate(_ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    Ok(())
}
