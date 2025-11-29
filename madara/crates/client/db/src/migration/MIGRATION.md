# Database Migration System

This document describes Madara's database migration system, which allows upgrading the database schema between versions without requiring a full resync.

## Overview

The migration system is inspired by [Pathfinder's approach](https://github.com/eqlabs/pathfinder/blob/main/crates/storage/src/schema.rs) but adapted for RocksDB.

### Key Features

- **Automatic version detection**: Detects database version on startup
- **Sequential migrations**: Applies migrations one by one in order
- **Pre-migration backups**: Creates a RocksDB checkpoint before migrating
- **Crash recovery**: Can resume interrupted migrations
- **Progress reporting**: Shows progress for long-running migrations

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      MigrationRunner                             │
│  - Checks current vs required version                           │
│  - Determines which migrations to run                           │
│  - Executes migrations sequentially                             │
│  - Handles crash recovery                                       │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Migration Registry                          │
│  - Maps version numbers to migration functions                  │
│  - Validates migration chain                                    │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Individual Migrations                       │
│  - revision_0009.rs: v8 -> v9                                   │
│  - revision_0010.rs: v9 -> v10                                  │
│  - ...                                                          │
└─────────────────────────────────────────────────────────────────┘
```

## Files

The migration system uses several files in the database directory:

| File                    | Purpose                                    |
| ----------------------- | ------------------------------------------ |
| `.db-version`           | Current database version (single integer)  |
| `.db-migration.lock`    | Lock file to prevent concurrent migrations |
| `.db-migration-state`   | Checkpoint file for crash recovery         |
| `backup_pre_migration/` | Pre-migration backup directory             |

## Adding a New Migration

### Step 1: Create the Migration File

Create a new file in `migration/revisions/`:

```rust
// migration/revisions/revision_0009.rs
//! Migration from v8 to v9: Description of changes

use crate::migration::{MigrationContext, MigrationError, MigrationProgress};

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    // Report initial progress
    ctx.report_progress(MigrationProgress::new(0, 3, "Starting migration"));

    // Step 1: Do something
    ctx.report_progress(MigrationProgress::new(1, 3, "Processing step 1"));
    // ... actual work ...

    // Check for abort periodically in long operations
    if ctx.should_abort() {
        return Err(MigrationError::Aborted);
    }

    // Step 2: Do something else
    ctx.report_progress(MigrationProgress::new(2, 3, "Processing step 2"));
    // ... actual work ...

    // Step 3: Finalize
    ctx.report_progress(MigrationProgress::new(3, 3, "Finalizing"));
    // ... actual work ...

    Ok(())
}
```

### Step 2: Export the Module

In `migration/revisions/mod.rs`:

```rust
pub mod revision_0009;
```

### Step 3: Register the Migration

In `migration/registry.rs`, add to `get_migrations()`:

```rust
pub fn get_migrations() -> &'static [Migration] {
    &[
        Migration {
            from_version: 8,
            to_version: 9,
            name: "my_migration_name",
            migrate: super::revisions::revision_0009::migrate,
        },
    ]
}
```

### Step 4: Update Version File

In `.db-versions.yml`:

```yaml
current_version: 9
base_version: 8 # Keep this if migrating from v8 is supported

versions:
  - version: 9
    pr: 123
    description: "Description of changes"
  # ... existing versions ...
```

## Migration Context

The `MigrationContext` provides:

```rust
impl MigrationContext {
    /// Get a reference to the RocksDB instance
    pub fn db(&self) -> &DB;

    /// Get the source version (migrating FROM)
    pub fn from_version(&self) -> u32;

    /// Get the target version (migrating TO)
    pub fn to_version(&self) -> u32;

    /// Report progress during migration
    pub fn report_progress(&self, progress: MigrationProgress);

    /// Check if the migration should be aborted
    pub fn should_abort(&self) -> bool;
}
```

## Error Handling

Migrations should return meaningful errors:

```rust
use crate::migration::MigrationError;

// For general failures
return Err(MigrationError::MigrationStepFailed {
    name: "migration_name".to_string(),
    from_version: 8,
    to_version: 9,
    message: "Detailed error message".to_string(),
});

// For user-requested abort
if ctx.should_abort() {
    return Err(MigrationError::Aborted);
}
```

## Recovery

### Failed Migration

If a migration fails:

1. The database may be in an inconsistent state
2. A backup exists at `<base_path>/backup_pre_migration/`
3. To restore:
   - Delete the `db/` directory
   - Rename `backup_pre_migration/` to `db/`
   - Update `.db-version` to the pre-migration version

### Interrupted Migration

If the node crashes during migration:

1. On next startup, the migration will be detected as interrupted
2. The system will resume from the last completed migration step
3. Completed steps are tracked in `.db-migration-state`

## Testing

### Unit Tests

Each migration should have unit tests in its revision file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_logic() {
        // Test migration logic
    }
}
```

### Integration Tests

Integration tests are in `tests/test_migration.rs`:

```rust
#[tokio::test]
async fn test_migration_from_v8_to_v9() {
    // Create database at v8
    // Run migration
    // Verify data integrity
}
```

## Best Practices

1. **Keep migrations small**: One logical change per migration
2. **Test thoroughly**: Both unit and integration tests
3. **Report progress**: For long-running operations
4. **Check abort flag**: Periodically in loops
5. **Use batches**: For large data operations
6. **Document changes**: In both code and version file

## Troubleshooting

### "Database version X is too old"

The database version is below `base_version`. You need to:

1. Delete the database directory
2. Resync from scratch

### "Database version X is newer than binary"

You're running an older binary with a newer database. You need to:

1. Upgrade to a newer binary version
2. Or delete and resync (data loss)

### "Migration is already in progress"

A lock file exists from a previous run. If no other instance is running:

1. Check `.db-migration.lock` file
2. If stale (>24h), it will be automatically removed
3. Or manually delete it

### Migration Failed

1. Check the error message in logs
2. The database may be in an inconsistent state
3. Restore from `backup_pre_migration/` if needed
