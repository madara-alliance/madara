//! Cache management for Cairo Native compiled classes
//!
//! This module handles both in-memory and disk-based caching of compiled native classes.

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::execution::native::contract_class::NativeCompiledClassV1;
use cairo_native::executor::AotContractExecutor;
use cairo_vm::types::errors::program_errors::ProgramError;
use dashmap::DashMap;
use mp_convert::ToFelt;
use starknet_api::core::ClassHash;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::config;
use mp_class::SierraConvertedClass;

/// Timeout for loading compiled classes from disk.
///
/// 2-second timeout prevents indefinite blocking when loading .so files.
///
/// **Why a timeout is necessary:**
/// 1. **Blocking contexts**: `try_get_from_disk_cache` can be called from blocking
///    contexts (transaction validation) where indefinite blocking is unacceptable.
///    Unlike CASM files loaded from RocksDB (which has its own timeout mechanisms),
///    disk file loading needs explicit timeout protection.
/// 2. **File system issues**: If the file system hangs (NFS, corrupted disk, etc.),
///    we need to fail fast and fall back to VM rather than blocking indefinitely.
/// 3. **Fast failure**: Shorter timeout ensures fast failure and fallback to VM if
///    disk load hangs, maintaining system responsiveness.
///
/// This is different from CASM file loading in the sequencer, which doesn't have
/// explicit timeouts because RocksDB handles timeouts internally. For disk files,
/// we need explicit timeout protection.
const DISK_LOAD_TIMEOUT: Duration = Duration::from_secs(2);

/// Timeout for memory cache lookup and conversion.
///
/// 100ms timeout prevents indefinite blocking during memory cache access or
/// conversion operations. While memory cache lookups should be instant, conversion
/// operations might hang due to issues with the cached native class.
const MEMORY_CACHE_TIMEOUT: Duration = Duration::from_millis(100);

/// In-memory cache for compiled native classes.
///
/// Uses DashMap for concurrent access without locking.
/// Stores tuples of `(compiled_class, last_access_time)` to enable LRU eviction.
/// Access time is updated on every cache hit to track recently used classes.
pub(crate) static NATIVE_CACHE: std::sync::LazyLock<DashMap<ClassHash, (Arc<NativeCompiledClassV1>, Instant)>> =
    std::sync::LazyLock::new(DashMap::new);

/// Get the cache file path for a class hash.
///
/// Constructs a path like `{cache_dir}/{class_hash:#x}.so`.
/// The filename is validated to prevent path traversal attacks.
pub(crate) fn get_native_cache_path(class_hash: &ClassHash, config: &config::NativeConfig) -> PathBuf {
    // Convert ClassHash to Felt for formatting (ClassHash implements ToFelt)
    let felt = class_hash.to_felt();
    // Ensure filename is safe (no path traversal)
    let filename = format!("{:#x}.so", felt);
    debug_assert!(!filename.contains("..") && !filename.contains("/"));

    config.cache_dir.join(filename)
}

/// Enforce disk cache size limit by removing least recently accessed files.
///
/// Sorts all `.so` files by access time (file modification time, updated on each access)
/// and removes the least recently accessed ones until the total size is below `max_size`.
/// If `max_size` is `None`, no limit is enforced.
///
/// **Access time tracking**: Files are "touched" (modification time updated) on each access
/// in `try_get_from_disk_cache`, so modification time effectively tracks access time.
/// This ensures LRU eviction based on actual usage, not just file creation time.
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
            // Use modification time as access time (files are touched on access)
            if let (Ok(modified), Ok(size)) = (metadata.modified(), metadata.len().try_into()) {
                entries.push((entry.path(), modified, size));
            }
        }
    }

    let total_size: u64 = entries.iter().map(|(_, _, size)| *size).sum();

    if total_size > max_size {
        let eviction_start = Instant::now();
        let bytes_to_remove = total_size - max_size;

        tracing::debug!(
            target: "madara_cairo_native",
            cache_dir = %cache_dir.display(),
            total_size_bytes = total_size,
            max_size_bytes = max_size,
            bytes_to_remove = bytes_to_remove,
            "disk_cache_eviction_start"
        );

        // Sort by access time (oldest first) - modification time tracks access time
        entries.sort_by_key(|(_, access_time, _)| *access_time);

        let mut to_remove = 0u64;
        let mut files_removed = 0usize;
        for (path, _, size) in entries {
            if to_remove >= bytes_to_remove {
                break;
            }
            std::fs::remove_file(&path)?;
            to_remove += size;
            files_removed += 1;
        }

        let eviction_elapsed = eviction_start.elapsed();

        tracing::debug!(
            target: "madara_cairo_native",
            cache_dir = %cache_dir.display(),
            removed_bytes = to_remove,
            removed_files = files_removed,
            remaining_bytes = total_size - to_remove,
            elapsed = ?eviction_elapsed,
            elapsed_ms = eviction_elapsed.as_millis(),
            "disk_cache_eviction_complete"
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
///
/// Requires config to be passed as a parameter (no global config fallback).
/// This function is only called when native config is present.
pub(crate) fn evict_cache_if_needed(config: &config::NativeConfig) {
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
    let eviction_start = Instant::now();

    tracing::debug!(
        target: "madara_cairo_native",
        current_size = current_size,
        max_size = max_size,
        evicting_count = to_remove,
        "memory_eviction_start"
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

    let eviction_elapsed = eviction_start.elapsed();

    // Cache size metric updated
    super::metrics::metrics().set_cache_size(NATIVE_CACHE.len());
    super::metrics::metrics().record_cache_eviction_time(eviction_elapsed.as_millis() as u64);

    tracing::debug!(
        target: "madara_cairo_native",
        evicted_count = to_remove,
        elapsed = ?eviction_elapsed,
        elapsed_ms = eviction_elapsed.as_millis(),
        "memory_eviction_complete"
    );
}

/// Try to get a class from the in-memory cache with a timeout.
///
/// Returns `Some(RunnableCompiledClass)` if found, `None` otherwise.
/// Updates access time for LRU tracking.
///
/// Wrapped in a timeout to prevent indefinite blocking during conversion.
pub(crate) fn try_get_from_memory_cache(class_hash: &ClassHash) -> Option<RunnableCompiledClass> {
    use std::sync::mpsc;
    use std::thread;

    let class_hash_for_log = *class_hash;
    let (tx, rx) = mpsc::channel();

    let thread_handle = thread::spawn(move || {
    let start = Instant::now();

        if let Some(cached_entry) = NATIVE_CACHE.get(&class_hash_for_log) {
            let (cached_class, _) = cached_entry.value();

            // Clone before dropping the reference to avoid holding lock during insert
            let cloned_class = cached_class.clone();
            drop(cached_entry); // Reference dropped before insert to minimize lock hold time

            // Access time updated for LRU eviction tracking
            NATIVE_CACHE.insert(class_hash_for_log, (cloned_class.clone(), Instant::now()));

            let lookup_elapsed = start.elapsed();
            let cache_size = NATIVE_CACHE.len();
            super::metrics::metrics().record_cache_hit_memory();
            super::metrics::metrics().record_cache_lookup_time_memory(lookup_elapsed.as_millis() as u64);

            // Convert Arc<NativeCompiledClassV1> to RunnableCompiledClass
            let conversion_start = Instant::now();
            let runnable = RunnableCompiledClass::from(cloned_class.as_ref().clone());
            let conversion_elapsed = conversion_start.elapsed();
            let total_elapsed = start.elapsed();

            super::metrics::metrics().record_cache_conversion_time(conversion_elapsed.as_millis() as u64);

            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash_for_log.to_felt()),
                elapsed = ?total_elapsed,
                lookup_ms = lookup_elapsed.as_millis(),
                conversion_ms = conversion_elapsed.as_millis(),
                cache_size = cache_size,
                "memory_hit"
            );

            let _ = tx.send(Some(runnable));
        } else {
            let _ = tx.send(None);
        }
    });

    match rx.recv_timeout(MEMORY_CACHE_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            super::metrics::metrics().record_cache_memory_timeout();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                timeout_ms = MEMORY_CACHE_TIMEOUT.as_millis(),
                "memory_cache_lookup_timeout"
            );
            // Don't wait for thread - it will be cleaned up when process exits
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            super::metrics::metrics().record_cache_memory_thread_disconnected();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                "memory_cache_thread_disconnected"
            );
            let _ = thread_handle.join();
            None
        }
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
        Ok(Ok(Some(executor))) => Ok(Some(executor)),
        Ok(Ok(None)) => {
            Ok(None) // File locked
        }
        Ok(Err(e)) => {
            let elapsed = start.elapsed();
            super::metrics::metrics().record_cache_disk_load_error();
            tracing::warn!(
                target: "madara_cairo_native",
                path = %path_for_logging.display(),
                error = %e,
                elapsed = ?elapsed,
                "disk_load_error"
            );
            Err(ProgramError::Parse(serde::de::Error::custom(format!("Failed to load executor: {}", e))))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let elapsed = start.elapsed();
            super::metrics::metrics().record_cache_disk_load_timeout();
            tracing::warn!(
                target: "madara_cairo_native",
                path = %path_for_logging.display(),
                timeout_secs = timeout.as_secs(),
                elapsed = ?elapsed,
                "disk_load_timeout"
            );
            // Note: Thread may still be running, but we can't wait for it
            // The thread will finish eventually, but we return None to avoid blocking
            // We don't join() here because if the thread is stuck, join() would also block indefinitely
            // The thread will be cleaned up when it finishes or when the process exits
            Ok(None) // Timeout - treat as cache miss
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let elapsed = start.elapsed();
            super::metrics::metrics().record_cache_disk_thread_disconnected();
            tracing::warn!(
                target: "madara_cairo_native",
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
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &config::NativeConfig,
) -> Result<Option<RunnableCompiledClass>, ProgramError> {
    let start = Instant::now();
    let path = get_native_cache_path(class_hash, config);


    if !path.exists() {
        let elapsed = start.elapsed();
        super::metrics::metrics().record_cache_disk_file_not_found();
        tracing::info!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            elapsed = ?elapsed,
            elapsed_ms = elapsed.as_millis(),
            "disk_cache_file_not_found"
        );
        return Ok(None);
    }

    // Check file size - if 0 or very small, file might be incomplete/corrupted
    if let Ok(metadata) = std::fs::metadata(&path) {
        let file_size = metadata.len();
        if file_size == 0 {
            super::metrics::metrics().record_cache_disk_file_empty();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                path = %path.display(),
                "disk_cache_file_empty_skipping"
            );
            return Ok(None);
        }
        tracing::info!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        path = %path.display(),
            file_size_bytes = file_size,
        "attempting_disk_load"
    );
    } else {
        super::metrics::metrics().record_cache_disk_metadata_error();
        tracing::warn!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            path = %path.display(),
            "disk_cache_file_metadata_error_skipping"
        );
        return Ok(None);
    }

    match try_load_executor_with_timeout(&path, DISK_LOAD_TIMEOUT)? {
        Some(executor) => {
            // Update file access time by touching it (updates modification time)
            // This ensures disk cache eviction uses access time, not creation time
            // Touch the file by opening for write (no-op) which updates modification time
            // This is a simple cross-platform way to update file access time for LRU eviction
            let _ = std::fs::OpenOptions::new().write(true).open(&path).and_then(|f| {
                // Truncate to same length (no-op) to update modification time
                if let Ok(metadata) = f.metadata() {
                    f.set_len(metadata.len()).ok();
                }
                Ok(())
            });

            let load_elapsed = start.elapsed();
            super::metrics::metrics().record_cache_hit_disk();
            super::metrics::metrics().record_cache_disk_load_time(load_elapsed.as_millis() as u64);

            // Converted to blockifier class using common logic
            let convert_start = Instant::now();
            let blockifier_compiled_class = match crate::compilation::convert_sierra_to_blockifier_class(sierra) {
                Ok(compiled) => compiled,
                Err(e) => {
                    tracing::error!(
                        target: "madara_cairo_native",
                        class_hash = %format!("{:#x}", class_hash.to_felt()),
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
            super::metrics::metrics().record_cache_disk_blockifier_convert_time(convert_elapsed.as_millis() as u64);

            let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

            // Insert first, then evict if needed (reduces lock contention)
            // This matches the old working implementation exactly
            NATIVE_CACHE.insert(*class_hash, (Arc::new(native_class.clone()), Instant::now()));

            // Evict if cache is full (after insert to reduce contention window)
            evict_cache_if_needed(config);

            // Convert to RunnableCompiledClass
            // Use the original native_class (not the Arc) for conversion
            // This matches the pattern used in blocking compilation
            let conversion_start = Instant::now();
            let runnable = RunnableCompiledClass::from(native_class);
            let conversion_elapsed = conversion_start.elapsed();
            let total_elapsed = start.elapsed();
            let cache_size = NATIVE_CACHE.len();

            super::metrics::metrics().record_cache_disk_runnable_convert_time(conversion_elapsed.as_millis() as u64);
            super::metrics::metrics().record_cache_lookup_time_disk(total_elapsed.as_millis() as u64);

            tracing::info!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                elapsed = ?total_elapsed,
                elapsed_ms = total_elapsed.as_millis(),
                load_ms = load_elapsed.as_millis(),
                convert_ms = convert_elapsed.as_millis(),
                conversion_ms = conversion_elapsed.as_millis(),
                cache_size = cache_size,
                "disk_hit"
            );
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
    use starknet_api::core::ClassHash;
    use starknet_types_core::felt::Felt;

    #[test]
    fn test_get_native_cache_path() {
        let felt = Felt::from(12345u64);
        let hash = ClassHash(felt);
        let config = config::NativeConfig::default();
        let path = get_native_cache_path(&hash, &config);
        assert!(path.to_string_lossy().contains("0x3039"));
        assert!(path.extension().and_then(|s| s.to_str()) == Some("so"));
    }

    #[test]
    fn test_get_native_cache_path_different_hashes() {
        let felt1 = Felt::from(12345u64);
        let felt2 = Felt::from(67890u64);
        let hash1 = ClassHash(felt1);
        let hash2 = ClassHash(felt2);
        let config = config::NativeConfig::default();
        let path1 = get_native_cache_path(&hash1, &config);
        let path2 = get_native_cache_path(&hash2, &config);

        // Different hashes should produce different paths
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_evict_cache_if_needed_empty() {
        // Clear cache first
        NATIVE_CACHE.clear();

        let config = config::NativeConfig::default();
        // This should not panic
        evict_cache_if_needed(&config);

        // Cache should still be empty
        assert_eq!(NATIVE_CACHE.len(), 0);
    }

    #[test]
    fn test_cache_size_metric() {
        // Clear cache
        NATIVE_CACHE.clear();

        let config = config::NativeConfig::default();
        // Cache operations should not panic
        evict_cache_if_needed(&config);
    }

    #[test]
    fn test_cache_eviction_logic() {
        // Clear cache
        NATIVE_CACHE.clear();

        let config = config::NativeConfig::default();
        let max_size = config.max_memory_cache_size;

        // Test that eviction threshold is calculated correctly
        // If Some, should be positive; if None, unlimited cache is valid
        if let Some(size) = max_size {
            assert!(size > 0, "Max cache size should be positive when set");
        }
        // None is also valid (unlimited cache)
    }
}
