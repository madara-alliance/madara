//! Migration registry - maps versions to migration functions.
//!
//! This module contains the list of all migrations and provides utilities
//! to find which migrations need to be applied.

use super::context::MigrationContext;
use super::error::MigrationError;

/// Type alias for migration functions.
///
/// A migration function takes a context and returns a Result.
/// On success, the migration is considered complete.
/// On failure, the entire migration process stops and the error is propagated.
pub type MigrationFn = fn(&MigrationContext<'_>) -> Result<(), MigrationError>;

/// Definition of a single migration.
#[derive(Clone)]
pub struct Migration {
    /// Version this migration starts from.
    pub from_version: u32,
    /// Version this migration upgrades to.
    pub to_version: u32,
    /// Human-readable name for logging.
    pub name: &'static str,
    /// The migration function to execute.
    pub migrate: MigrationFn,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("from_version", &self.from_version)
            .field("to_version", &self.to_version)
            .field("name", &self.name)
            .finish()
    }
}

/// Returns all registered migrations.
///
/// Migrations must be in ascending order by `from_version`.
/// Each migration should upgrade exactly one version (from_version + 1 = to_version).
///
/// # Adding a new migration
///
/// 1. Create a new file in `revisions/` (e.g., `revision_0010.rs`)
/// 2. Implement the `migrate` function
/// 3. Add the migration to this array
/// 4. Update `.db-versions.yml` with the new version
pub fn get_migrations() -> &'static [Migration] {
    &[
        // Add migrations here as they are implemented.
        // Example:
        // Migration {
        //     from_version: 8,
        //     to_version: 9,
        //     name: "snip34_blake_casm_migration",
        //     migrate: super::revisions::revision_0009::migrate,
        // },
    ]
}

/// Get migrations needed to upgrade from `from_version` to `to_version`.
///
/// Returns migrations in the order they should be applied.
/// Returns an empty Vec if no migrations are needed.
///
/// # Errors
///
/// Returns `MigrationError::NoMigrationPath` if there's a gap in the migration chain.
pub fn get_migrations_for_range(
    from_version: u32,
    to_version: u32,
) -> Result<Vec<&'static Migration>, MigrationError> {
    if from_version >= to_version {
        return Ok(vec![]);
    }

    let all_migrations = get_migrations();

    // Find migrations that fall within our range
    let mut migrations: Vec<_> = all_migrations
        .iter()
        .filter(|m| m.from_version >= from_version && m.to_version <= to_version)
        .collect();

    // Sort by from_version to ensure correct order
    migrations.sort_by_key(|m| m.from_version);

    // Validate the migration chain is complete
    if !migrations.is_empty() {
        let mut expected_version = from_version;
        for migration in &migrations {
            if migration.from_version != expected_version {
                return Err(MigrationError::NoMigrationPath { from: expected_version, to: migration.from_version });
            }
            expected_version = migration.to_version;
        }

        // Check we reach the target version
        if expected_version != to_version {
            return Err(MigrationError::NoMigrationPath { from: expected_version, to: to_version });
        }
    } else if from_version != to_version {
        // No migrations but we need to go from from_version to to_version
        return Err(MigrationError::NoMigrationPath { from: from_version, to: to_version });
    }

    Ok(migrations)
}

/// Validate the migration registry.
///
/// Checks that:
/// - Migrations are in order
/// - Each migration increments version by 1
/// - No duplicate versions
///
/// This should be called in tests to ensure the registry is valid.
pub fn validate_registry() -> Result<(), String> {
    let migrations = get_migrations();

    for (i, migration) in migrations.iter().enumerate() {
        // Check version increment
        if migration.to_version != migration.from_version + 1 {
            return Err(format!(
                "Migration '{}' has invalid version increment: {} -> {}",
                migration.name, migration.from_version, migration.to_version
            ));
        }

        // Check ordering
        if i > 0 {
            let prev = &migrations[i - 1];
            if migration.from_version != prev.to_version {
                return Err(format!(
                    "Migration gap: '{}' ends at v{} but '{}' starts at v{}",
                    prev.name, prev.to_version, migration.name, migration.from_version
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_is_valid() {
        // This ensures the migration registry is correctly configured
        validate_registry().expect("Migration registry should be valid");
    }

    #[test]
    fn test_get_migrations_same_version() {
        let migrations = get_migrations_for_range(8, 8).unwrap();
        assert!(migrations.is_empty());
    }

    #[test]
    fn test_get_migrations_from_greater_than_to() {
        let migrations = get_migrations_for_range(10, 8).unwrap();
        assert!(migrations.is_empty());
    }

    // More tests will be added once we have actual migrations
}

