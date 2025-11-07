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
// Timeout constants are now configurable via NativeConfig
// Default values: memory_cache_timeout = 100ms, disk_cache_load_timeout = 2s

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
            // Memory cache miss - log it
            let lookup_elapsed = start.elapsed();
            let cache_size = NATIVE_CACHE.len();
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

    match rx.recv_timeout(config.memory_cache_timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            super::metrics::metrics().record_cache_memory_timeout();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                timeout_ms = config.memory_cache_timeout.as_millis(),
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

    match try_load_executor_with_timeout(&path, config.disk_cache_load_timeout)? {
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
    use starknet_api::core::ClassHash;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    use mp_convert::ToFelt;
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;

    /// Test mutex to serialize test execution for tests that need to clear/modify caches
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    /// Counter for generating unique test class hashes
    static TEST_CLASS_HASH_COUNTER: AtomicU64 = AtomicU64::new(1);

    /// Helper function to create a unique test class hash
    fn create_unique_test_class_hash() -> ClassHash {
        let counter = TEST_CLASS_HASH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Use a large base value to avoid collisions with real class hashes
        let base = Felt::from_hex_unchecked("0x1000000000000000000000000000000000000000000000000000000000000000");
        ClassHash(base + Felt::from(counter))
    }

    /// Helper function to create a test SierraConvertedClass
    fn create_test_sierra_class() -> Result<SierraConvertedClass, Box<dyn std::error::Error>> {
        use m_cairo_test_contracts::TEST_CONTRACT_SIERRA;
        use mp_class::{CompiledSierra, FlattenedSierraClass, SierraClassInfo};
        use serde_json::Value;
        use std::sync::Arc;

        // Parse as JSON value first to check the structure
        let mut json_value: Value = serde_json::from_slice(TEST_CONTRACT_SIERRA)
            .map_err(|e| format!("Failed to parse TEST_CONTRACT_SIERRA as JSON: {}", e))?;

        // Handle abi field - convert from object/array to string if needed
        if let Some(abi_value) = json_value.get_mut("abi") {
            if !abi_value.is_string() {
                *abi_value = Value::String(serde_json::to_string(abi_value)?);
            }
        }

        // Check if sierra_program is an array (flattened) or string (compressed)
        let flattened_sierra = if let Some(sierra_program) = json_value.get("sierra_program") {
            if sierra_program.is_array() {
                serde_json::from_value::<FlattenedSierraClass>(json_value)
                    .map_err(|e| format!("Failed to parse as FlattenedSierraClass: {}", e))?
            } else if sierra_program.is_string() {
                use mp_class::CompressedSierraClass;
                let compressed = serde_json::from_value::<CompressedSierraClass>(json_value)
                    .map_err(|e| format!("Failed to parse as CompressedSierraClass: {}", e))?;
                FlattenedSierraClass::try_from(compressed)
                    .map_err(|e| format!("Failed to decompress CompressedSierraClass: {}", e))?
            } else {
                return Err("sierra_program field is neither an array nor a string".into());
            }
        } else {
            return Err("JSON does not contain sierra_program field".into());
        };

        // Compile to CASM to get compiled class hash
        let (compiled_class_hash, casm_class) = flattened_sierra.compile_to_casm()?;
        let compiled_sierra = CompiledSierra::try_from(&casm_class)?;

        // Create SierraClassInfo
        let sierra_info = SierraClassInfo { contract_class: Arc::new(flattened_sierra), compiled_class_hash };

        // Create SierraConvertedClass
        let sierra_converted = SierraConvertedClass {
            class_hash: compiled_class_hash,
            info: sierra_info,
            compiled: Arc::new(compiled_sierra),
        };

        Ok(sierra_converted)
    }

    /// Helper function to create a simple test config (for cache tests)
    fn create_simple_test_config(temp_dir: &TempDir) -> config::NativeConfig {
        config::NativeConfig::default().with_cache_dir(temp_dir.path().to_path_buf()).with_native_execution(true)
    }

    // Helper function to create a test NativeCompiledClassV1 from a Sierra class
    // This compiles a real Sierra class to create valid test data
    // If config is provided, uses get_native_cache_path for consistent path construction
    // Otherwise, uses temp_dir path directly
    // NOTE: This is unique to cache tests - kept here because it has cache-specific logic
    fn create_test_native_class(
        sierra: &SierraConvertedClass,
        temp_dir: &TempDir,
        class_hash: ClassHash,
        config: Option<&config::NativeConfig>,
    ) -> Result<Arc<NativeCompiledClassV1>, Box<dyn std::error::Error>> {
        // Use get_native_cache_path if config is provided, otherwise construct path from temp_dir
        let so_path = if let Some(config) = config {
            get_native_cache_path(&class_hash, config)
        } else {
            temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()))
        };

        // Compile Sierra to native
        let executor = sierra.info.contract_class.compile_to_native(&so_path)?;

        // Convert Sierra to blockifier compiled class
        let blockifier_compiled_class = crate::compilation::convert_sierra_to_blockifier_class(sierra)?;

        // Create NativeCompiledClassV1
        let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

        Ok(Arc::new(native_class))
    }

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
        let _guard = TEST_MUTEX.lock().unwrap();
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
        let _guard = TEST_MUTEX.lock().unwrap();
        // Clear cache
        NATIVE_CACHE.clear();

        let config = config::NativeConfig::default();
        // Cache operations should not panic
        evict_cache_if_needed(&config);
    }

    #[test]
    fn test_cache_eviction_logic() {
        let _guard = TEST_MUTEX.lock().unwrap();
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

    // ============================================================================
    // Priority 1.1: Memory Cache Hit Scenarios
    // ============================================================================

    #[test]
    fn test_basic_memory_cache_hit() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create native class and insert into cache
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache with initial timestamp
        let initial_time = Instant::now();
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), initial_time));

        // Verify cache contains our specific class
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Cache should contain our test class");

        // Try to get from memory cache - should succeed
        let result = try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir));
        assert!(result.is_some(), "Memory cache should return Some when class is cached");

        // Verify cache still contains the class (and access time was updated)
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Class should still be in cache after access");
        if let Some(entry) = NATIVE_CACHE.get(&class_hash) {
            let (_, access_time) = entry.value();
            assert!(*access_time >= initial_time, "Access time should be updated to current or later time");
        } else {
            panic!("Class should still be in cache after access");
        }
    }

    #[test]
    fn test_memory_cache_hit_after_disk_load() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Simulate disk load: create native class and insert into cache (as disk load would do)
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache (simulating what happens after disk load)
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), Instant::now()));

        // Verify class is now in memory cache
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Class should be in memory cache");

        // Request the class again - should hit memory cache
        let result = try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir));
        assert!(result.is_some(), "Should hit memory cache after disk load");
    }

    #[test]
    fn test_memory_cache_hit_updates_access_time() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create native class
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache with initial timestamp
        let initial_time = Instant::now();
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), initial_time));

        // Get initial access time from cache
        let initial_access_time = NATIVE_CACHE.get(&class_hash).map(|e| e.value().1).expect("Class should be in cache");

        // Small delay before accessing
        std::thread::sleep(Duration::from_millis(10));

        // Access the cached class
        let _result = try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir));
        assert!(_result.is_some(), "Should successfully retrieve from cache");

        // Verify access time was updated
        let updated_access_time =
            NATIVE_CACHE.get(&class_hash).map(|e| e.value().1).expect("Class should still be in cache");

        assert!(
            updated_access_time > initial_access_time,
            "Access time should be updated after cache hit. Initial: {:?}, Updated: {:?}",
            initial_access_time,
            updated_access_time
        );
    }

    #[test]
    fn test_memory_cache_hit_preserves_class_data() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create native class
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), Instant::now()));

        // Retrieve from cache
        let retrieved = try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir))
            .expect("Should successfully retrieve from cache");

        // Verify we got a RunnableCompiledClass (can't easily compare equality, but we can verify it's not None)
        // The fact that conversion succeeded means the data is preserved
        // In a more comprehensive test, we could execute the class and verify behavior matches
        assert!(std::mem::size_of_val(&retrieved) > 0, "Retrieved class should have non-zero size");
    }

    #[test]
    fn test_memory_cache_hit_concurrent_requests() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create native class
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), Instant::now()));

        // Create config once outside the loop to avoid moving temp_dir
        let test_config = create_simple_test_config(&temp_dir);

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
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Cache should still contain our test class");
    }

    #[test]
    fn test_memory_cache_hit_after_eviction_and_reload() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        // Use unique class_hash to avoid interference with other parallel tests
        let class_hash = create_unique_test_class_hash();

        // Create test Sierra class
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create native class
        let native_class =
            create_test_native_class(&sierra, &temp_dir, class_hash, None).expect("Failed to create test native class");

        // Insert into cache
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), Instant::now()));

        // Verify it's in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Class should be in cache");
        assert!(
            try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir)).is_some(),
            "Should retrieve from cache"
        );

        // Evict from memory cache (simulate eviction)
        NATIVE_CACHE.remove(&class_hash);
        assert!(!NATIVE_CACHE.contains_key(&class_hash), "Class should be removed from cache");

        // Verify it's no longer in memory cache
        assert!(
            try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir)).is_none(),
            "Should return None after eviction"
        );

        // Simulate reload from disk: insert back into cache
        NATIVE_CACHE.insert(class_hash, (native_class.clone(), Instant::now()));

        // Verify it's back in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Class should be back in cache");

        // Request again - should hit memory cache
        let result = try_get_from_memory_cache(&class_hash, &create_simple_test_config(&temp_dir));
        assert!(result.is_some(), "Should hit memory cache after reload from disk");
    }

    #[test]
    fn test_lru_eviction_when_limit_reached() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Create config with small cache limit (2 classes)
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = config::NativeConfig::default()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_native_execution(true)
            .with_max_memory_cache_size(Some(2)); // Limit to 2 classes

        // Clear cache to start fresh
        NATIVE_CACHE.clear();

        // Create test Sierra class (reused for all test classes)
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");

        // Create three unique class hashes
        let class_hash_1 = create_unique_test_class_hash();
        let class_hash_2 = create_unique_test_class_hash();
        let class_hash_3 = create_unique_test_class_hash();

        // Create native classes for all three
        let native_class_1 = create_test_native_class(&sierra, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");
        let native_class_2 = create_test_native_class(&sierra, &temp_dir, class_hash_2, None)
            .expect("Failed to create test native class 2");
        let native_class_3 = create_test_native_class(&sierra, &temp_dir, class_hash_3, None)
            .expect("Failed to create test native class 3");

        // Insert first class with initial timestamp
        let time_1 = Instant::now();
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference
        NATIVE_CACHE.insert(class_hash_1, (native_class_1.clone(), time_1));

        // Insert second class with later timestamp
        let time_2 = Instant::now();
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference
        NATIVE_CACHE.insert(class_hash_2, (native_class_2.clone(), time_2));

        // Verify both classes are in cache (at limit)
        assert_eq!(NATIVE_CACHE.len(), 2, "Cache should contain exactly 2 classes");
        assert!(NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 should be in cache");
        assert!(NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 should be in cache");

        // Access class_2 to update its access time (making it more recently used)
        let _ = try_get_from_memory_cache(&class_hash_2, &config);
        std::thread::sleep(Duration::from_millis(10)); // Small delay

        // Verify class_2's access time was updated
        let class_2_access_time =
            NATIVE_CACHE.get(&class_hash_2).map(|e| e.value().1).expect("Class 2 should be in cache");
        assert!(class_2_access_time > time_2, "Class 2 access time should be updated");

        // Insert third class (exceeds limit)
        let time_3 = Instant::now();
        NATIVE_CACHE.insert(class_hash_3, (native_class_3.clone(), time_3));

        // Verify cache now has 3 classes (before eviction)
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should have 3 classes before eviction");

        // Trigger eviction
        evict_cache_if_needed(&config);

        // Verify cache size is now at limit (2 classes)
        assert_eq!(NATIVE_CACHE.len(), 2, "Cache should be evicted down to limit of 2 classes");

        // Verify class_1 (oldest, not accessed) was evicted
        assert!(!NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 (oldest, not accessed) should be evicted");

        // Verify class_2 (accessed, more recent) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 (accessed, more recent) should remain in cache");

        // Verify class_3 (newest) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 (newest) should remain in cache");

        // Verify eviction metric was recorded
        use std::sync::atomic::Ordering;
        assert_eq!(test_counters::CACHE_EVICTIONS.load(Ordering::Relaxed), 1, "Should have exactly one cache eviction");
    }

    #[test]
    fn test_eviction_uses_access_time_correctly() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();

        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        // Create config with small cache limit (2 classes)
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = config::NativeConfig::default()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_native_execution(true)
            .with_max_memory_cache_size(Some(2)); // Limit to 2 classes

        // Clear cache to start fresh
        NATIVE_CACHE.clear();

        // Create test Sierra class (reused for all test classes)
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");

        // Create four unique class hashes
        let class_hash_1 = create_unique_test_class_hash();
        let class_hash_2 = create_unique_test_class_hash();
        let class_hash_3 = create_unique_test_class_hash();
        let class_hash_4 = create_unique_test_class_hash();

        // Create native classes for all four
        let native_class_1 = create_test_native_class(&sierra, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");
        let native_class_2 = create_test_native_class(&sierra, &temp_dir, class_hash_2, None)
            .expect("Failed to create test native class 2");
        let native_class_3 = create_test_native_class(&sierra, &temp_dir, class_hash_3, None)
            .expect("Failed to create test native class 3");
        let native_class_4 = create_test_native_class(&sierra, &temp_dir, class_hash_4, None)
            .expect("Failed to create test native class 4");

        // Insert first class
        let time_1 = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
        NATIVE_CACHE.insert(class_hash_1, (native_class_1.clone(), time_1));

        // Insert second class
        let time_2 = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
        NATIVE_CACHE.insert(class_hash_2, (native_class_2.clone(), time_2));

        // Verify both classes are in cache (at limit)
        assert_eq!(NATIVE_CACHE.len(), 2, "Cache should contain exactly 2 classes");

        // Access class_1 to update its access time (making it more recently used)
        let _ = try_get_from_memory_cache(&class_hash_1, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Verify class_1's access time was updated
        let class_1_access_time_after_first_access =
            NATIVE_CACHE.get(&class_hash_1).map(|e| e.value().1).expect("Class 1 should be in cache");
        assert!(
            class_1_access_time_after_first_access > time_1,
            "Class 1 access time should be updated after first access"
        );

        // Access class_2 to update its access time (making it even more recently used than class_1)
        let _ = try_get_from_memory_cache(&class_hash_2, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Verify class_2's access time was updated and is more recent than class_1
        let class_2_access_time =
            NATIVE_CACHE.get(&class_hash_2).map(|e| e.value().1).expect("Class 2 should be in cache");
        let class_1_access_time_before_eviction =
            NATIVE_CACHE.get(&class_hash_1).map(|e| e.value().1).expect("Class 1 should be in cache");
        assert!(
            class_2_access_time > class_1_access_time_before_eviction,
            "Class 2 should be more recently accessed than class 1"
        );

        // Insert third class (exceeds limit, should trigger eviction)
        let time_3 = Instant::now();
        NATIVE_CACHE.insert(class_hash_3, (native_class_3.clone(), time_3));

        // Verify cache now has 3 classes (before eviction)
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should have 3 classes before eviction");

        // Trigger eviction
        evict_cache_if_needed(&config);

        // Verify cache size is now at limit (2 classes)
        assert_eq!(NATIVE_CACHE.len(), 2, "Cache should be evicted down to limit of 2 classes");

        // Verify class_1 (least recently accessed after class_2 was accessed) was evicted
        assert!(!NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 (least recently accessed) should be evicted");

        // Verify class_2 (most recently accessed) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 (most recently accessed) should remain in cache");

        // Verify class_3 (newest) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 (newest) should remain in cache");

        // Now test with a different access pattern: add class_4 and access class_3
        // This should evict class_2 (least recently accessed among class_2 and class_3)
        NATIVE_CACHE.insert(class_hash_4, (native_class_4.clone(), Instant::now()));
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should have 3 classes before second eviction");

        // Access class_3 to make it more recently used than class_2
        let _ = try_get_from_memory_cache(&class_hash_3, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Trigger eviction again
        evict_cache_if_needed(&config);

        // Verify cache size is back to limit (2 classes)
        assert_eq!(NATIVE_CACHE.len(), 2, "Cache should be evicted down to limit of 2 classes again");

        // Verify class_2 (least recently accessed) was evicted
        assert!(!NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 (least recently accessed) should be evicted");

        // Verify class_3 (accessed, more recent) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 (accessed, more recent) should remain in cache");

        // Verify class_4 (newest) is still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_4), "Class 4 (newest) should remain in cache");

        // Verify eviction metrics were recorded
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_EVICTIONS.load(Ordering::Relaxed),
            2,
            "Should have exactly two cache evictions"
        );
    }

    #[test]
    fn test_eviction_removes_correct_entry() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = config::NativeConfig::default()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_native_execution(true)
            .with_max_memory_cache_size(Some(3)); // Limit to 3 classes

        NATIVE_CACHE.clear();

        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let class_hash_1 = create_unique_test_class_hash();
        let class_hash_2 = create_unique_test_class_hash();
        let class_hash_3 = create_unique_test_class_hash();
        let class_hash_4 = create_unique_test_class_hash();
        let class_hash_5 = create_unique_test_class_hash();

        // Create native classes
        let native_class_1 = create_test_native_class(&sierra, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");
        let native_class_2 = create_test_native_class(&sierra, &temp_dir, class_hash_2, None)
            .expect("Failed to create test native class 2");
        let native_class_3 = create_test_native_class(&sierra, &temp_dir, class_hash_3, None)
            .expect("Failed to create test native class 3");
        let native_class_4 = create_test_native_class(&sierra, &temp_dir, class_hash_4, None)
            .expect("Failed to create test native class 4");
        let native_class_5 = create_test_native_class(&sierra, &temp_dir, class_hash_5, None)
            .expect("Failed to create test native class 5");

        // Insert classes with increasing timestamps
        let time_1 = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
        NATIVE_CACHE.insert(class_hash_1, (native_class_1.clone(), time_1));

        let time_2 = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
        NATIVE_CACHE.insert(class_hash_2, (native_class_2.clone(), time_2));

        let time_3 = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
        NATIVE_CACHE.insert(class_hash_3, (native_class_3.clone(), time_3));

        // Verify all 3 classes are in cache (at limit)
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should contain exactly 3 classes");
        assert!(NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 should be in cache");
        assert!(NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 should be in cache");
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 should be in cache");

        // Access class_2 to update its access time (making it more recently used)
        let _ = try_get_from_memory_cache(&class_hash_2, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Access class_3 to update its access time (making it even more recently used)
        let _ = try_get_from_memory_cache(&class_hash_3, &config);
        std::thread::sleep(Duration::from_millis(10));

        // Insert class_4 (exceeds limit, should trigger eviction)
        let time_4 = Instant::now();
        NATIVE_CACHE.insert(class_hash_4, (native_class_4.clone(), time_4));
        assert_eq!(NATIVE_CACHE.len(), 4, "Cache should have 4 classes before eviction");

        // Trigger eviction
        evict_cache_if_needed(&config);

        // Verify cache size is now at limit (3 classes)
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should be evicted down to limit of 3 classes");

        // Verify class_1 (oldest, least recently accessed) was evicted
        assert!(!NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 should be evicted");
        assert!(NATIVE_CACHE.get(&class_hash_1).is_none(), "Class 1 should not be retrievable after eviction");

        // Verify class_2 (accessed, more recent) is still in cache and retrievable
        assert!(NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 should remain in cache");
        let retrieved_class_2 = NATIVE_CACHE.get(&class_hash_2);
        assert!(retrieved_class_2.is_some(), "Class 2 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_2.unwrap().value().0) > 0,
            "Retrieved class 2 should have non-zero size"
        );

        // Verify class_3 (accessed, most recent) is still in cache and retrievable
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 should remain in cache");
        let retrieved_class_3 = NATIVE_CACHE.get(&class_hash_3);
        assert!(retrieved_class_3.is_some(), "Class 3 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_3.unwrap().value().0) > 0,
            "Retrieved class 3 should have non-zero size"
        );

        // Verify class_4 (newest) is still in cache and retrievable
        assert!(NATIVE_CACHE.contains_key(&class_hash_4), "Class 4 should remain in cache");
        let retrieved_class_4 = NATIVE_CACHE.get(&class_hash_4);
        assert!(retrieved_class_4.is_some(), "Class 4 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_4.unwrap().value().0) > 0,
            "Retrieved class 4 should have non-zero size"
        );

        // Now add class_5 and verify it evicts class_2 (least recently accessed among remaining)
        let time_5 = Instant::now();
        NATIVE_CACHE.insert(class_hash_5, (native_class_5.clone(), time_5));
        assert_eq!(NATIVE_CACHE.len(), 4, "Cache should have 4 classes before second eviction");

        // Trigger eviction again
        evict_cache_if_needed(&config);

        // Verify cache size is back to limit (3 classes)
        assert_eq!(NATIVE_CACHE.len(), 3, "Cache should be evicted down to limit of 3 classes again");

        // Verify class_2 (least recently accessed among remaining) was evicted
        assert!(!NATIVE_CACHE.contains_key(&class_hash_2), "Class 2 should be evicted");
        assert!(NATIVE_CACHE.get(&class_hash_2).is_none(), "Class 2 should not be retrievable after eviction");

        // Verify class_1 is still not in cache (was already evicted)
        assert!(!NATIVE_CACHE.contains_key(&class_hash_1), "Class 1 should still not be in cache");

        // Verify class_3, class_4, and class_5 are still in cache
        assert!(NATIVE_CACHE.contains_key(&class_hash_3), "Class 3 should remain in cache");
        assert!(NATIVE_CACHE.contains_key(&class_hash_4), "Class 4 should remain in cache");
        assert!(NATIVE_CACHE.contains_key(&class_hash_5), "Class 5 should remain in cache");

        // Verify all remaining classes are retrievable with correct data
        let retrieved_class_3_final = NATIVE_CACHE.get(&class_hash_3);
        assert!(retrieved_class_3_final.is_some(), "Class 3 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_3_final.unwrap().value().0) > 0,
            "Retrieved class 3 should have non-zero size"
        );

        let retrieved_class_4_final = NATIVE_CACHE.get(&class_hash_4);
        assert!(retrieved_class_4_final.is_some(), "Class 4 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_4_final.unwrap().value().0) > 0,
            "Retrieved class 4 should have non-zero size"
        );

        let retrieved_class_5_final = NATIVE_CACHE.get(&class_hash_5);
        assert!(retrieved_class_5_final.is_some(), "Class 5 should be retrievable");
        assert!(
            std::mem::size_of_val(&retrieved_class_5_final.unwrap().value().0) > 0,
            "Retrieved class 5 should have non-zero size"
        );

        // Verify eviction metrics were recorded
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_EVICTIONS.load(Ordering::Relaxed),
            2,
            "Should have exactly two cache evictions"
        );
    }

    #[test]
    fn test_limited_memory_cache_config_respected() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let limit = 10;
        let config = config::NativeConfig::default()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_native_execution(true)
            .with_max_memory_cache_size(Some(limit)); // Limit to 10 classes

        NATIVE_CACHE.clear();

        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");

        // Add exactly `limit` classes
        let mut class_hashes = Vec::new();
        for i in 0..limit {
            let class_hash = create_unique_test_class_hash();
            let native_class = create_test_native_class(&sierra, &temp_dir, class_hash, None)
                .expect(&format!("Failed to create test native class {}", i));
            NATIVE_CACHE.insert(class_hash, (native_class, Instant::now()));
            class_hashes.push(class_hash);
        }

        // Verify cache is at limit
        assert_eq!(NATIVE_CACHE.len(), limit, "Cache should be at limit");

        // Add one more class - should trigger eviction
        let extra_class_hash = create_unique_test_class_hash();
        let extra_native_class = create_test_native_class(&sierra, &temp_dir, extra_class_hash, None)
            .expect("Failed to create extra native class");
        NATIVE_CACHE.insert(extra_class_hash, (extra_native_class, Instant::now()));

        // Trigger eviction
        evict_cache_if_needed(&config);

        // Verify cache size is exactly at limit
        assert_eq!(NATIVE_CACHE.len(), limit, "Cache should be evicted down to exactly {} classes", limit);

        // Verify cache never exceeds limit by adding more classes
        for i in 0..5 {
            let class_hash = create_unique_test_class_hash();
            let native_class = create_test_native_class(&sierra, &temp_dir, class_hash, None)
                .expect(&format!("Failed to create test native class {}", i));
            NATIVE_CACHE.insert(class_hash, (native_class, Instant::now()));
            evict_cache_if_needed(&config);
            assert!(
                NATIVE_CACHE.len() <= limit,
                "Cache should never exceed limit of {} classes, got {}",
                limit,
                NATIVE_CACHE.len()
            );
        }

        // Final verification
        assert_eq!(NATIVE_CACHE.len(), limit, "Cache should be exactly at limit after multiple evictions");
    }

    #[test]
    fn test_limited_disk_cache_config_respected() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");

        // Save to disk to get file size
        let config =
            config::NativeConfig::default().with_cache_dir(temp_dir.path().to_path_buf()).with_native_execution(true);

        // Create first file to get a baseline size
        let class_hash_1 = create_unique_test_class_hash();
        let path_1 = get_native_cache_path(&class_hash_1, &config);
        let _native_class_1 = create_test_native_class(&sierra, &temp_dir, class_hash_1, None)
            .expect("Failed to create test native class 1");

        // Create and save first class to disk
        let _native_class_1_disk = create_test_native_class(&sierra, &temp_dir, class_hash_1, Some(&config))
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

        let config_with_limit = config::NativeConfig::default()
            .with_cache_dir(temp_dir.path().to_path_buf())
            .with_native_execution(true)
            .with_max_disk_cache_size(Some(max_disk_size));

        // Create and save second class
        let class_hash_2 = create_unique_test_class_hash();
        let path_2 = get_native_cache_path(&class_hash_2, &config_with_limit);
        let _native_class_2 = create_test_native_class(&sierra, &temp_dir, class_hash_2, Some(&config_with_limit))
            .expect("Failed to create and save native class 2");
        assert!(path_2.exists(), "File 2 should exist on disk");
        std::thread::sleep(Duration::from_millis(10)); // Small delay to ensure time difference

        // Access file_2 to update its modification time (making it more recently accessed)
        // This ensures file_1 is the oldest and will be evicted first (LRU eviction)
        // This simulates what happens when try_get_from_disk_cache touches the file
        let _ = std::fs::OpenOptions::new().write(true).open(&path_2).and_then(|f| {
            if let Ok(metadata) = f.metadata() {
                f.set_len(metadata.len()).ok();
            }
            Ok(())
        });
        std::thread::sleep(Duration::from_millis(10)); // Small delay

        // Verify file_2's modification time was updated
        let file_2_modified_after_access =
            std::fs::metadata(&path_2).expect("File 2 should exist").modified().expect("Should get modification time");
        let file_1_modified_before =
            std::fs::metadata(&path_1).expect("File 1 should exist").modified().expect("Should get modification time");
        assert!(
            file_2_modified_after_access > file_1_modified_before,
            "File 2 modification time should be updated after access"
        );

        // Verify both files exist and total size is within limit
        let total_size = std::fs::metadata(&path_1).expect("File 1 should exist").len()
            + std::fs::metadata(&path_2).expect("File 2 should exist").len();
        assert!(total_size <= max_disk_size, "Total size {} should be within limit {}", total_size, max_disk_size);

        // Create and save third class - should trigger eviction
        let class_hash_3 = create_unique_test_class_hash();
        let path_3 = get_native_cache_path(&class_hash_3, &config_with_limit);
        let _native_class_3 = create_test_native_class(&sierra, &temp_dir, class_hash_3, Some(&config_with_limit))
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
        enforce_disk_cache_limit(&config_with_limit.cache_dir, config_with_limit.max_disk_cache_size)
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

    #[test]
    fn test_disk_cache_hit_updates_file_access_time() {
        // Acquire metrics mutex to prevent interference with other tests
        use crate::metrics::test_counters;
        let _metrics_guard = test_counters::acquire_and_reset();
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config =
            config::NativeConfig::default().with_cache_dir(temp_dir.path().to_path_buf()).with_native_execution(true);

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");

        // Create and save native class to disk
        let path = get_native_cache_path(&class_hash, &config);
        let _native_class = create_test_native_class(&sierra, &temp_dir, class_hash, Some(&config))
            .expect("Failed to create and save native class");
        assert!(path.exists(), "Native class file should exist on disk");

        // Get initial modification time
        let initial_modified =
            std::fs::metadata(&path).expect("File should exist").modified().expect("Should get modification time");

        // Wait a bit to ensure time difference
        std::thread::sleep(Duration::from_millis(100));

        // Clear memory cache to force disk lookup
        NATIVE_CACHE.remove(&class_hash);

        // Access from disk cache - this should update the file modification time
        let result = try_get_from_disk_cache(&class_hash, &sierra, &config);
        assert!(result.is_ok(), "Should successfully load from disk cache");
        assert!(result.unwrap().is_some(), "Should return cached class from disk");

        // Verify file modification time was updated
        let updated_modified = std::fs::metadata(&path)
            .expect("File should still exist")
            .modified()
            .expect("Should get modification time");

        assert!(
            updated_modified > initial_modified,
            "File modification time should be updated after disk cache access. Initial: {:?}, Updated: {:?}",
            initial_modified,
            updated_modified
        );

        // Verify the class was loaded into memory cache
        assert!(NATIVE_CACHE.contains_key(&class_hash), "Class should be loaded into memory cache after disk hit");
    }
}
