//! Migration from v12 to v13: clear Cairo Native disk cache.

use crate::migration::{MigrationContext, MigrationError, MigrationProgress};
use std::path::Path;

const NATIVE_CACHE_DIR: &str = "native_classes";

pub fn migrate(ctx: &MigrationContext<'_>) -> Result<(), MigrationError> {
    tracing::info!("Starting v12→v13 migration: clearing Cairo Native disk cache");
    ctx.report_progress(MigrationProgress::new(0, 2, "Locating Cairo Native disk cache"));

    if ctx.should_abort() {
        return Err(MigrationError::Aborted);
    }

    let native_cache_path = ctx.base_path().join(NATIVE_CACHE_DIR);
    ctx.report_progress(MigrationProgress::new(1, 2, "Clearing Cairo Native disk cache"));
    let cleared = clear_native_cache_path(&native_cache_path)?;

    if cleared {
        tracing::info!(
            target: "madara_db_migration",
            path = %native_cache_path.display(),
            "cleared_cairo_native_disk_cache"
        );
    } else {
        tracing::info!(
            target: "madara_db_migration",
            path = %native_cache_path.display(),
            "cairo_native_disk_cache_not_present"
        );
    }

    ctx.report_progress(MigrationProgress::new(2, 2, "Cairo Native disk cache cleared"));
    Ok(())
}

fn clear_native_cache_path(path: &Path) -> Result<bool, MigrationError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_native_cache_path_removes_existing_directory() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let native_cache_path = temp_dir.path().join(NATIVE_CACHE_DIR);
        std::fs::create_dir_all(&native_cache_path).expect("native cache dir should be created");
        std::fs::write(native_cache_path.join("0x1.so"), b"native").expect("native artifact should be written");
        std::fs::write(native_cache_path.join("0x1.so.meta.json"), b"metadata")
            .expect("metadata artifact should be written");

        let cleared = clear_native_cache_path(&native_cache_path).expect("native cache should be cleared");

        assert!(cleared);
        assert!(!native_cache_path.exists());
    }

    #[test]
    fn clear_native_cache_path_succeeds_when_directory_is_absent() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let native_cache_path = temp_dir.path().join(NATIVE_CACHE_DIR);

        let cleared = clear_native_cache_path(&native_cache_path).expect("missing native cache should not error");

        assert!(!cleared);
    }
}
