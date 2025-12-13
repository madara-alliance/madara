//! Database schema revisions.
//!
//! Files: `revision_XXXX.rs` where XXXX is target version (e.g., `revision_0009.rs` = v8→v9).
//!
//! To add a new revision:
//! 1. Create `revision_XXXX.rs` with `pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError>`
//! 2. Export module here
//! 3. Register in `registry::get_migrations()`
//! 4. Update `.db-versions.yml`

pub mod revision_0009;
