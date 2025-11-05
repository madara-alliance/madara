//! Cache management for Cairo Native compiled classes
//!
//! This module handles both in-memory and disk-based caching of compiled native classes.

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::execution::native::contract_class::NativeCompiledClassV1;
use cairo_native::executor::AotContractExecutor;
use cairo_vm::types::errors::program_errors::ProgramError;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::config;
use crate::SierraConvertedClass;

/// Timeout for loading compiled classes from disk.
///
/// 2-second timeout prevents indefinite blocking when loading .so files.
/// Important when called from blocking contexts like transaction validation.
/// Shorter timeout ensures fast failure and fallback to VM if disk load hangs.
const DISK_LOAD_TIMEOUT: Duration = Duration::from_secs(2);

/// In-memory cache for compiled native classes.
///
/// Uses DashMap for concurrent access without locking.
/// Stores tuples of `(compiled_class, last_access_time)` to enable LRU eviction.
/// Access time is updated on every cache hit to track recently used classes.
pub(crate) static NATIVE_CACHE: std::sync::LazyLock<
    DashMap<starknet_types_core::felt::Felt, (Arc<NativeCompiledClassV1>, Instant)>,
> = std::sync::LazyLock::new(DashMap::new);

/// Get the cache file path for a class hash.
///
/// Constructs a path like `{cache_dir}/{class_hash:#x}.so`.
/// The filename is validated to prevent path traversal attacks.
pub(crate) fn get_native_cache_path(
    class_hash: &starknet_types_core::felt::Felt,
    override_config: Option<&config::NativeConfig>,
) -> PathBuf {
    let config = override_config.unwrap_or_else(|| config::get_config());

    static LOGGED_PATH: std::sync::Once = std::sync::Once::new();
    LOGGED_PATH.call_once(|| {
        tracing::info!(
            target: "madara.cairo_native",
            cache_dir = %config.cache_dir.display(),
            "cache_directory_initialized"
        );
    });

    // Ensure filename is safe (no path traversal)
    let filename = format!("{:#x}.so", class_hash);
    debug_assert!(!filename.contains("..") && !filename.contains("/"));

    config.cache_dir.join(filename)
}

/// Enforce disk cache size limit by removing oldest files.
///
/// Sorts all `.so` files by modification time and removes the oldest ones until
/// the total size is below `max_size`. If `max_size` is `None`, no limit is enforced.
///
/// Called after each successful disk write to prevent unbounded growth.
pub(crate) fn enforce_disk_cache_limit(cache_dir: &PathBuf, max_size: Option<u64>) -> Result<(), std::io::Error> {
    let Some(max_size) = max_size else {
        return Ok(()); // No limit
    };

    let mut entries: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();

    for entry in std::fs::read_dir(cache_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            if let (Ok(modified), Ok(size)) = (metadata.modified(), metadata.len().try_into()) {
                entries.push((entry.path(), modified, size));
            }
        }
    }

    let total_size: u64 = entries.iter().map(|(_, _, size)| *size).sum();

    if total_size > max_size {
        // Sort by modification time (oldest first)
        entries.sort_by_key(|(_, mtime, _)| *mtime);

        let mut to_remove = 0u64;
        for (path, _, size) in entries {
            if to_remove >= total_size - max_size {
                break;
            }
            std::fs::remove_file(&path)?;
            to_remove += size;
        }

        tracing::info!(
            target: "madara.cairo_native",
            cache_dir = %cache_dir.display(),
            removed_bytes = to_remove,
            remaining_bytes = total_size - to_remove,
            "disk_cache_enforced"
        );
    }

    Ok(())
}

/// Evict entries from the memory cache if it exceeds the size limit.
///
/// Uses LRU (Least Recently Used) eviction based on access time.
/// Sorts all cache entries by access time (oldest first) and removes entries
/// until the cache size is within the configured limit.
///
/// **Thread Safety**: This function collects all entries quickly, then removes them.
/// To minimize lock contention, we collect keys first, then remove them in a separate step.
/// This prevents holding DashMap locks while doing expensive operations.
///
/// If `max_memory_cache_size` is `None`, no eviction is performed (unlimited cache).
pub(crate) fn evict_cache_if_needed() {
    let config = config::get_config();

    let Some(max_size) = config.max_memory_cache_size else {
        // Update cache size metric only, no eviction
        super::metrics::metrics().set_cache_size(NATIVE_CACHE.len());
        return; // No limit
    };

    // Quick size check first (fast operation)
    let current_size = NATIVE_CACHE.len();
    if current_size <= max_size {
        // Update cache size metric only
        super::metrics::metrics().set_cache_size(current_size);
        return;
    }

    let to_remove = current_size - max_size;
    tracing::info!(
    target: "madara.cairo_native",
    current_size = current_size,
    max_size = max_size,
        evicting_count = to_remove,
    "memory_eviction"
    );

    // Keys and access times collected quickly to minimize lock hold time
    // Access times cloned to avoid holding references during iteration
    let mut entries: Vec<_> = {
        NATIVE_CACHE
            .iter()
            .map(|entry| {
                let key = *entry.key();
                let (_, access_time) = entry.value();
                (key, *access_time) // Clone only the access time, not the entire class
            })
            .collect()
    };

    // Sorted by access time (oldest first) - fast operation, no locks held
    entries.sort_by_key(|(_, access_time)| *access_time);

    // Oldest entries removed (fast removals, no expensive cloning)
    for (key, _) in entries.into_iter().take(to_remove) {
        if NATIVE_CACHE.remove(&key).is_some() {
            super::metrics::metrics().record_cache_eviction();
        }
    }

    // Cache size metric updated
    super::metrics::metrics().set_cache_size(NATIVE_CACHE.len());
}

/// Try to get a class from the in-memory cache.
///
/// Returns `Some(RunnableCompiledClass)` if found, `None` otherwise.
/// Updates access time for LRU tracking.
pub(crate) fn try_get_from_memory_cache(class_hash: &starknet_types_core::felt::Felt) -> Option<RunnableCompiledClass> {
    let start = Instant::now();

    if let Some(cached_entry) = NATIVE_CACHE.get(class_hash) {
        let (cached_class, _) = cached_entry.value();

        // Clone before dropping the reference to avoid holding lock during insert
        let cloned_class = cached_class.clone();
        drop(cached_entry); // Reference dropped before insert to minimize lock hold time

        // Access time updated for LRU eviction tracking
        NATIVE_CACHE.insert(*class_hash, (cloned_class.clone(), Instant::now()));

        let elapsed = start.elapsed();
        let cache_size = NATIVE_CACHE.len();
        super::metrics::metrics().record_cache_hit_memory();

        tracing::debug!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            elapsed = ?elapsed,
            cache_size = cache_size,
            "memory_hit"
        );

        let runnable = RunnableCompiledClass::from(cloned_class.as_ref().clone());
        Some(runnable)
    } else {
        None
    }
}

/// Try to load a class from disk cache with a timeout.
///
/// Wraps `AotContractExecutor::from_path` in a timeout to prevent indefinite blocking
/// when called from blocking contexts (e.g., during transaction validation).
fn try_load_executor_with_timeout(
    path: &std::path::Path,
    timeout: Duration,
) -> Result<Option<AotContractExecutor>, ProgramError> {
    use std::sync::mpsc;
    use std::thread;

    // Clone path for use in error messages (after moving into closure)
    let path_for_logging = path.to_path_buf();
    let path_for_thread = path.to_path_buf();
    let (tx, rx) = mpsc::channel();

    // Spawn a thread to load the executor
    let thread_handle = thread::spawn(move || {
        let result = AotContractExecutor::from_path(&path_for_thread);
        let _ = tx.send(result);
    });

    // Wait for result with timeout
    let start = Instant::now();
    match rx.recv_timeout(timeout) {
        Ok(Ok(Some(executor))) => {
            let elapsed = start.elapsed();
            tracing::debug!(
                target: "madara.cairo_native",
                path = %path_for_logging.display(),
                elapsed = ?elapsed,
                "disk_load_success"
            );
            Ok(Some(executor))
        }
        Ok(Ok(None)) => {
            Ok(None) // File locked
        }
        Ok(Err(e)) => {
            let elapsed = start.elapsed();
            tracing::warn!(
                target: "madara.cairo_native",
                path = %path_for_logging.display(),
                error = %e,
                elapsed = ?elapsed,
                "disk_load_error"
            );
            Err(ProgramError::Parse(serde::de::Error::custom(format!("Failed to load executor: {}", e))))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let elapsed = start.elapsed();
            tracing::warn!(
                target: "madara.cairo_native",
                path = %path_for_logging.display(),
                timeout_secs = timeout.as_secs(),
                elapsed = ?elapsed,
                "disk_load_timeout"
            );
            // Note: Thread may still be running, but we can't wait for it
            // The thread will finish eventually, but we return None to avoid blocking
            Ok(None) // Timeout - treat as cache miss
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let elapsed = start.elapsed();
            tracing::warn!(
                target: "madara.cairo_native",
                path = %path_for_logging.display(),
                elapsed = ?elapsed,
                "disk_load_thread_disconnected"
            );
            // Thread panicked or disconnected - treat as cache miss
            // Drop the thread handle to clean up
            let _ = thread_handle.join();
            Ok(None)
        }
    }
}

/// Try to load a class from disk cache.
///
/// Returns `Some(RunnableCompiledClass)` if successfully loaded, `None` otherwise.
/// On success, also adds the class to the memory cache.
pub(crate) fn try_get_from_disk_cache(
    class_hash: &starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
) -> Result<Option<RunnableCompiledClass>, ProgramError> {
    let start = Instant::now();
    let path = get_native_cache_path(class_hash, None);

    if !path.exists() {
        return Ok(None);
    }

    match try_load_executor_with_timeout(&path, DISK_LOAD_TIMEOUT)? {
        Some(executor) => {
            let load_elapsed = start.elapsed();
            super::metrics::metrics().record_cache_hit_disk();

            // Converted to blockifier class using common logic
            let convert_start = Instant::now();
            let blockifier_compiled_class = match crate::native::compilation::convert_sierra_to_blockifier_class(sierra)
            {
                Ok(compiled) => compiled,
                Err(e) => {
                    tracing::error!(
                        target: "madara.cairo_native",
                        class_hash = %format!("{:#x}", class_hash),
                        error = %e,
                        "disk_hit_conversion_failed"
                    );
                    // Corrupted file removal attempted
                    let _ = std::fs::remove_file(&path);
                    return Err(ProgramError::Parse(serde::de::Error::custom(format!(
                        "Failed to convert cached class: {}",
                        e
                    ))));
                }
            };
            let convert_elapsed = convert_start.elapsed();

            let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

            // Insert first, then evict if needed (reduces lock contention)
            NATIVE_CACHE.insert(*class_hash, (Arc::new(native_class.clone()), Instant::now()));

            // Evict if cache is full (after insert to reduce contention window)
            evict_cache_if_needed();

            let total_elapsed = start.elapsed();
            let cache_size = NATIVE_CACHE.len();

            tracing::info!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                elapsed = ?total_elapsed,
                load = ?load_elapsed,
                convert = ?convert_elapsed,
                cache_size = cache_size,
                "disk_hit"
            );

            let runnable = RunnableCompiledClass::from(native_class);
            Ok(Some(runnable))
        }
        None => {
            // Timeout or file locked - treat as cache miss
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_types_core::felt::Felt;

    #[test]
    fn test_get_native_cache_path() {
        let hash = Felt::from(12345u64);
        let path = get_native_cache_path(&hash, None);
        assert!(path.to_string_lossy().contains("0x3039"));
        assert!(path.extension().and_then(|s| s.to_str()) == Some("so"));
    }

    #[test]
    fn test_get_native_cache_path_different_hashes() {
        let hash1 = Felt::from(12345u64);
        let hash2 = Felt::from(67890u64);
        let path1 = get_native_cache_path(&hash1, None);
        let path2 = get_native_cache_path(&hash2, None);

        // Different hashes should produce different paths
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_evict_cache_if_needed_empty() {
        // Clear cache first
        NATIVE_CACHE.clear();

        // This should not panic
        evict_cache_if_needed();

        // Cache should still be empty
        assert_eq!(NATIVE_CACHE.len(), 0);
    }

    #[test]
    fn test_cache_size_metric() {
        // Clear cache
        NATIVE_CACHE.clear();

        // Cache operations should not panic
        evict_cache_if_needed();
    }

    #[test]
    fn test_cache_eviction_logic() {
        // Clear cache
        NATIVE_CACHE.clear();

        let config = super::config::get_config();
        let max_size = config.max_memory_cache_size;

        // Test that eviction threshold is calculated correctly
        // If Some, should be positive; if None, unlimited cache is valid
        if let Some(size) = max_size {
            assert!(size > 0, "Max cache size should be positive when set");
        }
        // None is also valid (unlimited cache)
    }
}
