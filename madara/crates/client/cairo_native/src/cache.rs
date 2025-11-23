//! Cache management for Cairo Native compiled classes
//!
//! This module handles both in-memory and disk-based caching of compiled native classes.

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_native::executor::AotContractExecutor;
use cairo_vm::types::errors::program_errors::ProgramError;
use dashmap::DashMap;
use mp_convert::ToFelt;
use starknet_api::core::ClassHash;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::config;
use super::native_class::NativeCompiledClass;
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
/// Timeout constants are now configurable via NativeConfig
/// Default values: memory_cache_timeout = 100ms, disk_cache_load_timeout = 2s
/// In-memory cache for compiled native classes.
///
/// Uses DashMap for concurrent access without locking.
/// Stores tuples of `(compiled_class, last_access_time)` to enable LRU eviction.
/// Access time is updated on every cache hit to track recently used classes.
///
/// **Access Control**: This cache is private. Use accessor functions to interact with it:
/// - `cache_contains()` - Check if a class exists
/// - `cache_insert()` - Insert a class (internal use)
/// - `cache_get()` - Get a class (tests only)
/// - `cache_remove()` - Remove a class (tests only)
/// - `cache_clear()` - Clear cache (tests only)
static NATIVE_CACHE: std::sync::LazyLock<DashMap<ClassHash, (NativeCompiledClass, Instant)>> =
    std::sync::LazyLock::new(DashMap::new);

/// Check if a class hash exists in the memory cache.
pub(crate) fn cache_contains(class_hash: &ClassHash) -> bool {
    NATIVE_CACHE.contains_key(class_hash)
}

/// Insert a class into the memory cache (internal use only).
///
/// This is used internally by cache operations. External code should use
/// `cache_compiled_native_class()` or other high-level functions.
pub(crate) fn cache_insert(class_hash: ClassHash, native_class: NativeCompiledClass, access_time: Instant) {
    NATIVE_CACHE.insert(class_hash, (native_class, access_time));
}

/// Get a class from cache (for tests only).
#[cfg(test)]
pub(crate) fn cache_get(class_hash: &ClassHash) -> Option<(NativeCompiledClass, Instant)> {
    NATIVE_CACHE.get(class_hash).map(|e| e.value().clone())
}

/// Remove a class from cache (for tests only).
#[cfg(test)]
pub(crate) fn cache_remove(class_hash: &ClassHash) {
    NATIVE_CACHE.remove(class_hash);
}

/// Clear the cache (for tests only).
#[cfg(test)]
pub(crate) fn cache_clear() {
    NATIVE_CACHE.clear();
}

/// Get the current cache size.
pub(crate) fn cache_len() -> usize {
    NATIVE_CACHE.len()
}

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

    config
        .execution_config()
        .expect("get_disk_cache_path should only be called when native execution is enabled")
        .cache_dir
        .join(filename)
}

/// Enforce disk cache size limit by removing least recently accessed files.
///
/// When the disk cache grows too large, this function removes the oldest unused files
/// to make room for new ones. Files are sorted by when they were last accessed,
/// and the oldest files are deleted until the cache size is within the limit.
///
/// **How it works**:
/// - Tracks when each cached file was last used
/// - When the cache exceeds the size limit, removes files that haven't been used recently
/// - This ensures frequently used classes stay in cache while rarely used ones are removed
/// - Works across different filesystems, automatically adapting to system capabilities
///
/// Called automatically after saving new files to disk to keep the cache size manageable.
pub(crate) fn enforce_disk_cache_limit(cache_dir: &PathBuf, max_size: Option<u64>) -> Result<(), std::io::Error> {
    let Some(max_size) = max_size else {
        return Ok(()); // No limit
    };

    let mut entries: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();

    for entry in std::fs::read_dir(cache_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            // Determine when this file was last accessed for LRU eviction
            // Try to use the system's access time first, but fall back to modification time
            // if access time tracking is disabled (common on modern filesystems for performance)
            // Since we update modification time whenever a file is accessed, it serves as a reliable
            // indicator of when the file was last used
            let access_time = metadata
                .accessed()
                .ok()
                .or_else(|| metadata.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            let size = metadata.len();
            entries.push((entry.path(), access_time, size));
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
    let exec_config = config
        .execution_config()
        .expect("evict_cache_if_needed should only be called when native execution is enabled");
    let Some(max_size) = exec_config.max_memory_cache_size else {
        // Update cache size metric only, no eviction
        super::metrics::metrics().set_cache_size(cache_len());
        return; // No limit
    };

    // Quick size check first (fast operation)
    let current_size = cache_len();
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
    super::metrics::metrics().set_cache_size(cache_len());
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
pub(crate) fn try_get_from_memory_cache(
    class_hash: &ClassHash,
    config: &config::NativeConfig,
) -> Option<RunnableCompiledClass> {
    use std::sync::mpsc;
    use std::thread;

    let class_hash_for_log = *class_hash;
    let (tx, rx) = mpsc::channel();

    let thread_handle = thread::spawn(move || {
        let start = Instant::now();

        if let Some(cached_entry) = NATIVE_CACHE.get(&class_hash_for_log) {
            // Retrieve the cached class (Arc clone is cheap - just increments reference count, doesn't copy data)
            let cached_class = cached_entry.value().0.clone();
            drop(cached_entry); // Release shard lock before expensive operations below

            // Update the access time to mark this class as recently used
            // This ensures the cache knows which classes are most frequently accessed
            // When the cache is full, least recently used classes will be evicted first
            // Note: Cloning Arc again here is cheap (just increments ref count), and we need a separate
            // reference to update the cache entry while keeping the original for conversion below
            cache_insert(class_hash_for_log, cached_class.clone(), Instant::now());

            let lookup_elapsed = start.elapsed();
            let cache_size = cache_len();
            super::metrics::metrics().record_cache_hit_memory();
            super::metrics::metrics().record_cache_lookup_time_memory(lookup_elapsed.as_millis() as u64);

            // Convert NativeCompiledClass to RunnableCompiledClass
            let conversion_start = Instant::now();
            let runnable = cached_class.to_runnable();
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
            // Memory cache miss - log it
            let lookup_elapsed = start.elapsed();
            let cache_size = cache_len();
            super::metrics::metrics().record_cache_memory_miss();
            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash_for_log.to_felt()),
                elapsed = ?lookup_elapsed,
                elapsed_ms = lookup_elapsed.as_millis(),
                cache_size = cache_size,
                "memory_cache_miss"
            );
            let _ = tx.send(None);
        }
    });

    let exec_config = config
        .execution_config()
        .expect("try_get_from_memory_cache should only be called when native execution is enabled");
    match rx.recv_timeout(exec_config.memory_cache_timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            super::metrics::metrics().record_cache_memory_timeout();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                timeout_ms = exec_config.memory_cache_timeout.as_millis(),
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
/// **Why a timeout?**
/// Unlike CASM file loading from the database (which is a simple read operation), loading native
/// compiled classes from disk involves:
/// - Dynamic library loading (`dlopen`/`LoadLibrary`) which can hang if the file is corrupted or locked
/// - File system I/O that may be slow or blocked by other processes
/// - Memory mapping operations that could block indefinitely on certain filesystems
///
/// **Comparison with CASM loading:**
/// - CASM files are loaded from RocksDB, which has built-in timeout handling and is optimized for fast reads
/// - Native `.so` files require OS-level dynamic linking, which has no built-in timeout mechanism
/// - A timeout prevents blocking transaction validation when disk I/O is slow or files are corrupted
///
/// **Timeout behavior:**
/// - Default timeout: 2 seconds (configurable via `disk_cache_load_timeout`)
/// - On timeout: Returns `None` (treated as cache miss), allowing fallback to compilation
/// - Prevents indefinite blocking during transaction validation, ensuring system responsiveness
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
///
/// **Guard Clause**: Skips disk cache if compilation is in progress to avoid loading incomplete files.
pub(crate) fn try_get_from_disk_cache(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &config::NativeConfig,
) -> Result<Option<RunnableCompiledClass>, ProgramError> {
    let start = Instant::now();

    // Skip disk cache if compilation is in progress to avoid loading incomplete files
    if super::compilation::is_compilation_in_progress(class_hash) {
        let elapsed = start.elapsed();
        super::metrics::metrics().record_compilation_in_progress_skip();
        tracing::debug!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            elapsed = ?elapsed,
            elapsed_ms = elapsed.as_millis(),
            "compilation_in_progress_skipping_disk_cache"
        );
        return Ok(None);
    }

    let path = get_native_cache_path(class_hash, config);

    if !path.exists() {
        let elapsed = start.elapsed();
        super::metrics::metrics().record_cache_disk_file_not_found();
        tracing::debug!(
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
        tracing::debug!(
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

    let exec_config = config
        .execution_config()
        .expect("try_get_from_disk_cache should only be called when native execution is enabled");
    match try_load_executor_with_timeout(&path, exec_config.disk_cache_load_timeout)? {
        Some(executor) => {
            // Mark this file as recently accessed by updating its modification time
            // This helps the cache eviction system know which files are actively being used
            // When the cache needs to free up space, it will remove files that haven't been
            // accessed recently, keeping frequently used classes available
            let _ = std::fs::OpenOptions::new().write(true).open(&path).map(|f| {
                // Update the file timestamp without changing its contents
                if let Ok(metadata) = f.metadata() {
                    f.set_len(metadata.len()).ok();
                }
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

            let native_class = NativeCompiledClass::new(executor, blockifier_compiled_class);

            // Insert first, then evict if needed (reduces lock contention)
            // This matches the old working implementation exactly
            cache_insert(*class_hash, native_class.clone(), Instant::now());

            // Evict if cache is full (after insert to reduce contention window)
            evict_cache_if_needed(config);

            // Convert to RunnableCompiledClass
            let conversion_start = Instant::now();
            let runnable = native_class.to_runnable();
            let conversion_elapsed = conversion_start.elapsed();
            let total_elapsed = start.elapsed();
            let cache_size = cache_len();

            super::metrics::metrics().record_cache_disk_runnable_convert_time(conversion_elapsed.as_millis() as u64);
            super::metrics::metrics().record_cache_lookup_time_disk(total_elapsed.as_millis() as u64);

            tracing::debug!(
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
    use mp_class::SierraConvertedClass;
    use rstest::rstest;
    use starknet_api::core::ClassHash;
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    use mp_convert::ToFelt;
    use std::sync::Mutex;
    // Import fixtures from test_utils
    use crate::test_utils::{sierra_class, temp_dir, test_config};

    // Import assert_counters macro (exported at crate root due to #[macro_export])
    use crate::assert_counters;

    /// Test mutex to serialize test execution for tests that need to clear/modify caches
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    /// Helper function to create a unique test class hash (module_id=1 for cache.rs)
    fn create_unique_test_class_hash() -> ClassHash {
        crate::test_utils::create_unique_test_class_hash(1)
    }

    // Helper function to create a test NativeCompiledClass from a Sierra class
    // This compiles a real Sierra class to create valid test data
    // If config is provided, uses get_native_cache_path for consistent path construction
    // Otherwise, uses temp_dir path directly
    // NOTE: This is unique to cache tests - kept here because it has cache-specific logic
    fn create_test_native_class(
        sierra: &SierraConvertedClass,
        temp_dir: &TempDir,
        class_hash: ClassHash,
        config: Option<&config::NativeConfig>,
    ) -> Result<NativeCompiledClass, Box<dyn std::error::Error>> {
        // Use get_native_cache_path if config is provided, otherwise construct path from temp_dir
        let so_path = if let Some(config) = config {
            get_native_cache_path(&class_hash, config)
        } else {
            temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()))
        };

        crate::test_utils::create_native_class_internal(sierra, &so_path)
    }

    #[rstest]
    fn test_basic_memory_cache_hit(
        sierra_class: SierraConvertedClass,
        temp_dir: TempDir,
        test_config: config::NativeConfig,
    ) {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create native class and insert into cache
        let native_class = create_test_native_class(&sierra_class, &temp_dir, class_hash, None)
            .expect("Failed to create test native class");

        // Insert into cache with initial timestamp
        let initial_time = Instant::now();
        cache_insert(class_hash, native_class.clone(), initial_time);

        // Verify cache contains our specific class
        assert!(cache_contains(&class_hash), "Cache should contain our test class");

        // Try to get from memory cache - should succeed
        let result = try_get_from_memory_cache(&class_hash, &test_config);
        assert!(result.is_some(), "Memory cache should return Some when class is cached");

        // Verify cache still contains the class (and access time was updated)
        assert!(cache_contains(&class_hash), "Class should still be in cache after access");
        if let Some((_, access_time)) = cache_get(&class_hash) {
            assert!(access_time > initial_time, "Access time should be updated to current or later time");
        } else {
            panic!("Class should still be in cache after access");
        }
    }

    #[rstest]
    fn test_memory_cache_hit_after_disk_load(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create config using the temp_dir fixture (so cache_dir matches)
        let test_config = crate::test_utils::create_test_config(&temp_dir, None, false);

        // Create and save native class to disk (simulate previous compilation)
        let path = get_native_cache_path(&class_hash, &test_config);
        let _native_class = create_test_native_class(&sierra_class, &temp_dir, class_hash, Some(&test_config))
            .expect("Failed to create and save native class to disk");
        assert!(path.exists(), "Native class file should exist on disk");

        // Clear memory cache to ensure we start fresh
        cache_remove(&class_hash);
        assert!(!cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Load from disk cache - this should populate memory cache
        let disk_result = try_get_from_disk_cache(&class_hash, &sierra_class, &test_config);
        assert!(disk_result.is_ok(), "Should successfully load from disk cache");
        assert!(disk_result.unwrap().is_some(), "Should return cached class from disk");

        // Verify class is now in memory cache after disk load
        assert!(cache_contains(&class_hash), "Class should be in memory cache after disk load");

        // Request the class again - should hit memory cache (not disk)
        let result = try_get_from_memory_cache(&class_hash, &test_config);
        assert!(result.is_some(), "Should hit memory cache after disk load");
    }

    #[rstest]
    fn test_memory_cache_hit_concurrent_requests(
        sierra_class: SierraConvertedClass,
        temp_dir: TempDir,
        test_config: config::NativeConfig,
    ) {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create native class
        let native_class = create_test_native_class(&sierra_class, &temp_dir, class_hash, None)
            .expect("Failed to create test native class");

        // Insert into cache
        cache_insert(class_hash, native_class.clone(), Instant::now());

        // Spawn multiple threads that all request the same cached class
        let num_threads = 10;
        let mut handles = vec![];

        for _ in 0..num_threads {
            let hash = class_hash;
            let config = test_config.clone(); // Clone config for each thread
            let handle = thread::spawn(move || try_get_from_memory_cache(&hash, &config));
            handles.push(handle);
        }

        // Collect results from all threads
        let mut results = vec![];
        for handle in handles {
            let result = handle.join().expect("Thread should not panic");
            results.push(result);
        }

        // Verify all threads got the same result (Some)
        assert_eq!(results.len(), num_threads);
        for result in &results {
            assert!(result.is_some(), "All threads should get Some from cache");
        }

        // Verify cache still contains our specific class
        assert!(cache_contains(&class_hash), "Cache should still contain our test class");
    }

    #[rstest]
    fn test_lru_eviction_when_limit_reached(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Create config with small cache limit (2 classes)
        let config = config::NativeConfig::builder()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_max_memory_cache_size(Some(2)) // Limit to 2 classes
            .build();

        // Clear cache to start fresh
        cache_clear();

        // Create four unique class hashes
        let class_hash_1 = create_unique_test_class_hash();
        let class_hash_2 = create_unique_test_class_hash();
        let class_hash_3 = create_unique_test_class_hash();
        let class_hash_4 = create_unique_test_class_hash();

        // Create native classes for all four
        let native_class_1 = create_test_native_class(&sierra_class, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");
        let native_class_2 = create_test_native_class(&sierra_class, &temp_dir, class_hash_2, None)
            .expect("Failed to create test native class 2");
        let native_class_3 = create_test_native_class(&sierra_class, &temp_dir, class_hash_3, None)
            .expect("Failed to create test native class 3");
        let native_class_4 = create_test_native_class(&sierra_class, &temp_dir, class_hash_4, None)
            .expect("Failed to create test native class 4");

        // ============================================================================
        // Phase 1: Basic LRU eviction - oldest unaccessed item is evicted
        // ============================================================================

        // Insert first class with initial timestamp
        let time_1 = Instant::now();
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference
        cache_insert(class_hash_1, native_class_1.clone(), time_1);

        // Insert second class with later timestamp
        let time_2 = Instant::now();
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference
        cache_insert(class_hash_2, native_class_2.clone(), time_2);

        // Verify both classes are in cache (at limit)
        assert_eq!(cache_len(), 2, "Cache should contain exactly 2 classes");
        assert!(cache_contains(&class_hash_1), "Class 1 should be in cache");
        assert!(cache_contains(&class_hash_2), "Class 2 should be in cache");

        // Access class_2 to update its access time (making it more recently used)
        let _ = try_get_from_memory_cache(&class_hash_2, &config);
        std::thread::sleep(Duration::from_millis(10)); // Small delay

        // Verify class_2's access time was updated
        let class_2_access_time = cache_get(&class_hash_2).map(|(_, time)| time).expect("Class 2 should be in cache");
        assert!(class_2_access_time > time_2, "Class 2 access time should be updated");

        // Insert third class (exceeds limit)
        let time_3 = Instant::now();
        cache_insert(class_hash_3, native_class_3.clone(), time_3);

        // Verify cache now has 3 classes (before eviction)
        assert_eq!(cache_len(), 3, "Cache should have 3 classes before eviction");

        // Trigger eviction
        evict_cache_if_needed(&config);

        // Verify cache size is now at limit (2 classes)
        assert_eq!(cache_len(), 2, "Cache should be evicted down to limit of 2 classes");

        // Verify class_1 (oldest, not accessed) was evicted
        assert!(!cache_contains(&class_hash_1), "Class 1 (oldest, not accessed) should be evicted");

        // Verify class_2 (accessed, more recent) is still in cache
        assert!(cache_contains(&class_hash_2), "Class 2 (accessed, more recent) should remain in cache");

        // Verify class_3 (newest) is still in cache
        assert!(cache_contains(&class_hash_3), "Class 3 (newest) should remain in cache");

        // ============================================================================
        // Phase 2: Access time tracking - verify access order affects eviction
        // ============================================================================

        // Access class_2 again to update its access time (making it more recently used than class_3)
        let _ = try_get_from_memory_cache(&class_hash_2, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Verify class_2's access time was updated and is more recent than class_3
        let class_2_access_time_after_second_access =
            cache_get(&class_hash_2).map(|(_, time)| time).expect("Class 2 should be in cache");
        let class_3_access_time_before_eviction =
            cache_get(&class_hash_3).map(|(_, time)| time).expect("Class 3 should be in cache");
        assert!(
            class_2_access_time_after_second_access > class_3_access_time_before_eviction,
            "Class 2 should be more recently accessed than class 3"
        );

        // Insert fourth class (exceeds limit, should trigger eviction)
        cache_insert(class_hash_4, native_class_4.clone(), Instant::now());
        assert_eq!(cache_len(), 3, "Cache should have 3 classes before second eviction");

        // Trigger eviction again
        evict_cache_if_needed(&config);

        // Verify cache size is back to limit (2 classes)
        assert_eq!(cache_len(), 2, "Cache should be evicted down to limit of 2 classes again");

        // Verify class_3 (least recently accessed among class_2 and class_3) was evicted
        assert!(!cache_contains(&class_hash_3), "Class 3 (least recently accessed) should be evicted");

        // Verify class_2 (most recently accessed) is still in cache
        assert!(cache_contains(&class_hash_2), "Class 2 (most recently accessed) should remain in cache");

        // Verify class_4 (newest) is still in cache
        assert!(cache_contains(&class_hash_4), "Class 4 (newest) should remain in cache");

        // Verify eviction metrics were recorded
        assert_counters!(
            CACHE_EVICTIONS: 2,
        );
    }

    #[rstest]
    fn test_limited_disk_cache_config_respected(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Save to disk to get file size
        let config = config::NativeConfig::builder().with_cache_dir(temp_dir.path().to_path_buf()).build();

        // Create first file to get a baseline size
        let class_hash_1 = create_unique_test_class_hash();
        let path_1 = get_native_cache_path(&class_hash_1, &config);
        let _native_class_1 = create_test_native_class(&sierra_class, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");

        // Create and save first class to disk
        let _native_class_1_disk = create_test_native_class(&sierra_class, &temp_dir, class_hash_1, Some(&config))
            .expect("Failed to create and save native class 1");
        assert!(path_1.exists(), "File 1 should exist on disk");
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference

        let file_size = std::fs::metadata(&path_1).expect("File 1 should exist").len();

        // Set disk cache limit to accommodate exactly 2 files (with some margin)
        // We want to ensure that deleting file_1 alone brings us under the limit
        // So: file_2 + file_3 <= max_disk_size, but file_1 + file_2 + file_3 > max_disk_size
        // This means: max_disk_size should be between (file_2 + file_3) and (file_1 + file_2 + file_3)
        // Since files are similar size, we can use: max_disk_size = file_size * 2 + file_size / 2
        // This ensures file_1 alone is enough to bring us under limit
        let max_disk_size = file_size * 2 + file_size / 2; // Allows 2 files + half a file

        let config_with_limit = config::NativeConfig::builder()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_max_disk_cache_size(Some(max_disk_size))
            .build();

        // Create and save second class
        let class_hash_2 = create_unique_test_class_hash();
        let path_2 = get_native_cache_path(&class_hash_2, &config_with_limit);
        let _native_class_2 =
            create_test_native_class(&sierra_class, &temp_dir, class_hash_2, Some(&config_with_limit))
                .expect("Failed to create and save native class 2");
        assert!(path_2.exists(), "File 2 should exist on disk");
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference

        // Access file_2 via try_get_from_disk_cache to update its modification time
        // This tests that try_get_from_disk_cache updates file access time (LRU tracking)
        // and ensures file_2 is more recently accessed than file_1 (for LRU eviction)
        cache_remove(&class_hash_2); // Clear memory cache to force disk lookup
        let initial_file_2_modified =
            std::fs::metadata(&path_2).expect("File 2 should exist").modified().expect("Should get modification time");
        std::thread::sleep(Duration::from_millis(100)); // Wait to ensure time difference

        let disk_result = try_get_from_disk_cache(&class_hash_2, &sierra_class, &config_with_limit);
        assert!(disk_result.is_ok(), "Should successfully load from disk cache");
        assert!(disk_result.unwrap().is_some(), "Should return cached class from disk");

        // Verify file_2's modification time was updated by try_get_from_disk_cache
        let file_2_modified_after_access =
            std::fs::metadata(&path_2).expect("File 2 should exist").modified().expect("Should get modification time");
        assert!(
            file_2_modified_after_access > initial_file_2_modified,
            "File 2 modification time should be updated after try_get_from_disk_cache access"
        );

        // Verify file_2 is more recently accessed than file_1
        let file_1_modified_before =
            std::fs::metadata(&path_1).expect("File 1 should exist").modified().expect("Should get modification time");
        assert!(
            file_2_modified_after_access > file_1_modified_before,
            "File 2 should be more recently accessed than file 1 (for LRU eviction)"
        );

        // Verify the class was loaded into memory cache after disk hit
        assert!(cache_contains(&class_hash_2), "Class should be loaded into memory cache after disk hit");

        // Verify both files exist and total size is within limit
        let total_size = std::fs::metadata(&path_1).expect("File 1 should exist").len()
            + std::fs::metadata(&path_2).expect("File 2 should exist").len();
        assert!(total_size <= max_disk_size, "Total size {} should be within limit {}", total_size, max_disk_size);

        // Create and save third class - should trigger eviction
        let class_hash_3 = create_unique_test_class_hash();
        let path_3 = get_native_cache_path(&class_hash_3, &config_with_limit);
        let _native_class_3 =
            create_test_native_class(&sierra_class, &temp_dir, class_hash_3, Some(&config_with_limit))
                .expect("Failed to create and save native class 3");
        assert!(path_3.exists(), "File 3 should exist on disk");

        // Get file sizes before eviction to verify files are complete after eviction
        let file_1_size = std::fs::metadata(&path_1).expect("File 1 should exist").len();
        let file_2_size = std::fs::metadata(&path_2).expect("File 2 should exist").len();
        let file_3_size = std::fs::metadata(&path_3).expect("File 3 should exist").len();

        // Verify total size exceeds limit (will trigger eviction)
        let total_size_before = file_1_size + file_2_size + file_3_size;
        assert!(
            total_size_before > max_disk_size,
            "Total size {} should exceed limit {} to trigger eviction",
            total_size_before,
            max_disk_size
        );

        // Verify file_2 + file_3 fit within limit (so only file_1 needs to be deleted)
        let file_2_plus_3_size = file_2_size + file_3_size;
        assert!(
            file_2_plus_3_size <= max_disk_size,
            "File 2 + File 3 size {} should be within limit {} (so only file_1 needs eviction)",
            file_2_plus_3_size,
            max_disk_size
        );

        // Verify file_1 is the oldest (will be evicted first) - LRU eviction based on access time
        let file_1_modified =
            std::fs::metadata(&path_1).expect("File 1 should exist").modified().expect("Should get modification time");
        let file_2_modified =
            std::fs::metadata(&path_2).expect("File 2 should exist").modified().expect("Should get modification time");
        let file_3_modified =
            std::fs::metadata(&path_3).expect("File 3 should exist").modified().expect("Should get modification time");

        assert!(file_1_modified < file_2_modified, "File 1 should be older than file 2 (will be evicted first)");
        assert!(file_1_modified < file_3_modified, "File 1 should be older than file 3 (will be evicted first)");

        // Trigger eviction with the configured limit
        let exec_config = config_with_limit.execution_config().expect("test should use enabled config");
        enforce_disk_cache_limit(&exec_config.cache_dir, exec_config.max_disk_cache_size)
            .expect("Disk cache eviction should succeed");

        // Verify file_1 (oldest, LRU) was completely deleted
        assert!(!path_1.exists(), "File 1 (oldest, LRU) should be completely deleted");

        // Verify file_2 and file_3 still exist and are complete (not truncated)
        assert!(path_2.exists(), "File 2 (more recently accessed) should still exist");
        assert!(path_3.exists(), "File 3 (newest) should still exist");

        let file_2_current_size = std::fs::metadata(&path_2).expect("File 2 should exist").len();
        let file_3_current_size = std::fs::metadata(&path_3).expect("File 3 should exist").len();

        assert_eq!(
            file_2_current_size, file_2_size,
            "File 2 should be complete (not truncated). Original size: {}, Current size: {}",
            file_2_size, file_2_current_size
        );
        assert_eq!(
            file_3_current_size, file_3_size,
            "File 3 should be complete (not truncated). Original size: {}, Current size: {}",
            file_3_size, file_3_current_size
        );

        // Verify total size is now within limit (config limit is respected)
        let remaining_size = file_2_size + file_3_size;
        assert!(
            remaining_size <= max_disk_size,
            "Remaining disk cache size {} should be within limit {}",
            remaining_size,
            max_disk_size
        );
    }
}
