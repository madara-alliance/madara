//! Database schema revisions.
//!
//! Each revision file contains a single migration that upgrades the database
//! from one version to the next.
//!
//! # Naming Convention
//!
//! Files are named `revision_XXXX.rs` where `XXXX` is the target version
//! (zero-padded to 4 digits). For example:
//! - `revision_0009.rs` - Migrates from v8 to v9
//! - `revision_0010.rs` - Migrates from v9 to v10
//!
//! # Adding a New Revision
//!
//! 1. Create a new file following the naming convention
//! 2. Implement the `migrate` function with signature:
//!    `pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError>`
//! 3. Export the module here
//! 4. Add the migration to `registry::get_migrations()`
//! 5. Update `.db-versions.yml`
//!
//! # Example Revision
//!
//! ```ignore
//! use crate::migration::{MigrationContext, MigrationError, MigrationProgress};
//!
//! pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
//!     // Report initial progress
//!     ctx.report_progress(MigrationProgress::new(0, 3, "Starting migration"));
//!
//!     // Step 1: Do something
//!     ctx.report_progress(MigrationProgress::new(1, 3, "Processing step 1"));
//!     // ... actual work ...
//!
//!     // Check for abort periodically in long operations
//!     if ctx.should_abort() {
//!         return Err(MigrationError::Aborted);
//!     }
//!
//!     // Step 2: Do something else
//!     ctx.report_progress(MigrationProgress::new(2, 3, "Processing step 2"));
//!     // ... actual work ...
//!
//!     // Step 3: Finalize
//!     ctx.report_progress(MigrationProgress::new(3, 3, "Finalizing"));
//!     // ... actual work ...
//!
//!     Ok(())
//! }
//! ```

// Export revision modules here as they are added:
pub mod revision_0009;
