//! Compilation orchestration for Cairo Native
//!
//! This module handles the compilation of Sierra classes to native code,
//! including blocking and async modes, retry logic, and concurrency control.

use cairo_native::executor::AotContractExecutor;
use dashmap::DashMap;
use mp_convert::ToFelt;
use starknet_api::core::ClassHash;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::config;
use super::error::NativeCompilationError;
use super::native_class::NativeCompiledClass;
use mp_class::SierraConvertedClass;

use super::cache;

/// Tracks which classes are currently being compiled to avoid duplicate compilations.
///
/// Uses RwLock entries to coordinate between multiple threads requesting the same class.
/// Lock is released after compilation completes (success or failure).
///
/// **Ownership**: Only `spawn_compilation_if_needed` inserts entries into this map.
/// All other functions only read from or remove from it. This ensures clear ownership
/// and prevents race conditions.
///
/// **Access Control**: This map is private. Use accessor functions to interact with it:
/// - `is_compilation_in_progress()` - Check if compilation is in progress
/// - `mark_compilation_in_progress()` - Mark compilation as in progress (internal use)
/// - `remove_compilation_in_progress()` - Remove compilation marker (internal use)
/// - `get_current_compilations_count()` - Get current count
static COMPILATION_IN_PROGRESS: std::sync::LazyLock<DashMap<ClassHash, Arc<RwLock<()>>>> =
    std::sync::LazyLock::new(DashMap::new);

/// Check if compilation is in progress for a class hash.
pub(crate) fn is_compilation_in_progress(class_hash: &ClassHash) -> bool {
    COMPILATION_IN_PROGRESS.contains_key(class_hash)
}

/// Mark compilation as in progress and return the lock (internal use only).
///
/// This function atomically checks if compilation is already in progress and marks it if not.
/// Returns `Some(lock)` if compilation was successfully marked (new compilation started),
/// or `None` if compilation was already in progress (duplicate request).
///
/// This is used internally by `spawn_compilation_if_needed` to prevent duplicate compilations.
pub(crate) fn mark_compilation_in_progress(class_hash: ClassHash) -> Option<Arc<RwLock<()>>> {
    use dashmap::mapref::entry::Entry;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    match COMPILATION_IN_PROGRESS.entry(class_hash) {
        Entry::Vacant(entry) => {
            // Not compiling - mark as in progress atomically
            let lock = Arc::new(RwLock::new(()));
            entry.insert(lock.clone());
            Some(lock)
        }
        Entry::Occupied(_) => {
            // Already compiling - return None to indicate duplicate
            None
        }
    }
}

/// Remove compilation in progress marker (internal use only).
pub(crate) fn remove_compilation_in_progress(class_hash: &ClassHash) {
    COMPILATION_IN_PROGRESS.remove(class_hash);
}

/// Insert compilation in progress marker (for tests only).
#[cfg(test)]
pub(crate) fn insert_compilation_in_progress(class_hash: ClassHash) {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    COMPILATION_IN_PROGRESS.insert(class_hash, Arc::new(RwLock::new(())));
}

/// Get compilation lock if compilation is in progress.
///
/// This can be used to wait on the compilation lock in blocking mode.
/// Returns `Some(lock)` if compilation is in progress, `None` otherwise.
pub(crate) fn get_compilation_lock(class_hash: &ClassHash) -> Option<Arc<RwLock<()>>> {
    COMPILATION_IN_PROGRESS.get(class_hash).map(|e| e.value().clone())
}

/// Get the current number of compilations in progress.
///
/// This is used by metrics to update the current compilations gauge.
pub(crate) fn get_current_compilations_count() -> usize {
    COMPILATION_IN_PROGRESS.len()
}

/// Clear all compilation in progress markers (for tests only).
#[cfg(test)]
pub(crate) fn clear_compilations_in_progress() {
    COMPILATION_IN_PROGRESS.clear();
}

/// Tracks classes that failed compilation in async mode.
///
/// **Purpose**: When a compilation fails in async mode, the class hash is added here with
/// a timestamp. This serves two purposes:
/// 1. **Logging**: Enables special logging on subsequent requests to help debug compilation issues
/// 2. **Retry Control**: When `config.enable_retry` is enabled, classes in this map are automatically
///    retried on the next request, allowing recovery from transient failures
///
/// **Retry Behavior**:
/// - If `config.enable_retry` is `true`: Classes in this map are automatically retried on next request
/// - If `config.enable_retry` is `false`: Classes in this map are NOT retried (prevents wasted compilation attempts)
///   - However, they are still tracked for logging purposes to help identify problematic classes
///
/// **Eviction**: Entries are evicted using LRU policy when the map exceeds a reasonable size limit
/// (similar to memory cache eviction). This prevents unbounded growth.
///
/// **When Retrying is Beneficial**:
/// - Previous failure was due to timeout (system was under heavy load)
/// - Temporary resource constraints (disk space, memory) have been resolved
/// - Transient file system issues have been resolved
/// - Race conditions that caused the initial failure have cleared
///
/// **Note**: Retrying doesn't guarantee success - if a class genuinely cannot be compiled (e.g., invalid Sierra),
/// retrying will fail again. The retry mechanism is primarily useful for transient failures.
///
/// **Storage**: Uses `Instant` to track when the failure occurred for LRU eviction.
/// Maximum size is limited to prevent unbounded growth (default: 10,000 entries).
///
/// **Access Control**: This map is private. Use accessor functions to interact with it:
/// - `has_failed_compilation()` - Check if a class failed compilation
/// - `mark_failed_compilation()` - Mark a class as failed (internal use)
/// - `remove_failed_compilation()` - Remove failed marker (internal use)
static FAILED_COMPILATIONS: std::sync::LazyLock<DashMap<ClassHash, Instant>> = std::sync::LazyLock::new(DashMap::new);

/// Check if a class hash has a failed compilation record.
pub(crate) fn has_failed_compilation(class_hash: &ClassHash) -> bool {
    FAILED_COMPILATIONS.contains_key(class_hash)
}

/// Mark a class as having failed compilation (internal use only).
pub(crate) fn mark_failed_compilation(class_hash: ClassHash, timestamp: Instant) {
    FAILED_COMPILATIONS.insert(class_hash, timestamp);
}

/// Remove a failed compilation marker (internal use only).
pub(crate) fn remove_failed_compilation(class_hash: &ClassHash) {
    FAILED_COMPILATIONS.remove(class_hash);
}

/// Clear all failed compilation records (for tests only).
#[cfg(test)]
pub(crate) fn clear_failed_compilations() {
    FAILED_COMPILATIONS.clear();
}

/// Evict entries from FAILED_COMPILATIONS if it exceeds the size limit.
///
/// Uses LRU (Least Recently Used) eviction based on failure timestamp.
/// Sorts all entries by timestamp (oldest first) and removes entries until
/// the map size is within the configured limit.
fn evict_failed_compilations_if_needed(max_failed_compilations: usize) {
    // Quick size check first (fast operation)
    let current_size = FAILED_COMPILATIONS.len();
    if current_size < max_failed_compilations {
        return;
    }

    let to_remove = current_size - max_failed_compilations + 1; // Remove one extra to make room
    let eviction_start = Instant::now();

    tracing::debug!(
        target: "madara_cairo_native",
        current_size = current_size,
        max_size = max_failed_compilations,
        evicting_count = to_remove,
        "failed_compilations_eviction_start"
    );

    // Collect keys and timestamps quickly to minimize lock hold time
    let mut entries: Vec<_> = {
        FAILED_COMPILATIONS
            .iter()
            .map(|entry| {
                let key = *entry.key();
                let timestamp = *entry.value();
                (key, timestamp)
            })
            .collect()
    };

    // Sort by timestamp (oldest first) - fast operation, no locks held
    entries.sort_by_key(|(_, timestamp)| *timestamp);

    // Remove oldest entries
    for (key, _) in entries.into_iter().take(to_remove) {
        FAILED_COMPILATIONS.remove(&key);
    }

    let eviction_elapsed = eviction_start.elapsed();

    tracing::debug!(
        target: "madara_cairo_native",
        evicted_count = to_remove,
        elapsed = ?eviction_elapsed,
        elapsed_ms = eviction_elapsed.as_millis(),
        "failed_compilations_eviction_complete"
    );
}

/// Semaphore to limit concurrent compilations.
///
/// Initialized with the actual config value during `setup_and_log()`.
/// Uses `OnceLock` so it can be set once at startup with the correct value.
/// If accessed before initialization, falls back to default value.
static COMPILATION_SEMAPHORE: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();

/// Initialize the compilation semaphore with the actual config value.
///
/// This must be called during config initialization (via `setup_and_log()`).
/// The semaphore limit controls how many compilations can run concurrently.
pub fn init_compilation_semaphore(max_concurrent: usize) {
    COMPILATION_SEMAPHORE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)));
}

/// Try to acquire a compilation permit, respecting the current config limit.
///
/// Returns `None` if the semaphore limit has been reached.
/// The semaphore should be initialized at startup via `init_compilation_semaphore()`.
/// If not initialized, falls back to default value (should never happen in normal operation).
fn try_acquire_compilation_permit() -> Option<tokio::sync::SemaphorePermit<'static>> {
    // Get semaphore, initializing if needed (should be initialized at startup, but handle fallback)
    let semaphore = COMPILATION_SEMAPHORE.get_or_init(|| {
        // Fallback: semaphore not initialized yet, use default value
        // This should never happen in normal operation since semaphore is initialized in main.rs
        tracing::warn!(
            target: "madara_cairo_native",
            "compilation_semaphore_not_initialized_using_default_limit"
        );
        Arc::new(tokio::sync::Semaphore::new(config::DEFAULT_MAX_CONCURRENT_COMPILATIONS))
    });

    semaphore.try_acquire().ok()
}

/// Convert Sierra contract class to blockifier compiled class.
///
/// This is shared logic used by both blocking and async compilation paths.
/// Performs the conversion: Sierra → CASM → Blockifier CompiledClassV1.
///
/// Returns a `NativeCompilationError` if any step in the conversion fails.
pub(crate) fn convert_sierra_to_blockifier_class(
    sierra: &SierraConvertedClass,
) -> Result<blockifier::execution::contract_class::CompiledClassV1, NativeCompilationError> {
    let sierra_version = sierra
        .info
        .contract_class
        .sierra_version()
        .map_err(|e| NativeCompilationError::SierraVersionError(e.to_string()))?;

    let casm: casm_classes_v2::casm_contract_class::CasmContractClass = sierra
        .compiled
        .as_ref()
        .try_into()
        .map_err(|e: serde_json::Error| NativeCompilationError::CasmConversionError(e.to_string()))?;

    let blockifier_compiled_class = (casm, sierra_version)
        .try_into()
        .map_err(|e| NativeCompilationError::BlockifierConversionError(format!("{:?}", e)))?;

    Ok(blockifier_compiled_class)
}

/// Execute native compilation with appropriate timeout handling.
///
/// Handles both async and blocking contexts, returning the executor or an error.
/// This is a pure function that only performs compilation - no caching or metrics.
fn execute_native_compilation(
    sierra: &SierraConvertedClass,
    path: &PathBuf,
    timeout: Duration,
    timer: super::metrics::CompilationTimer,
) -> Result<AotContractExecutor, NativeCompilationError> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // Async context detected - spawn_blocking used with timeout
        let sierra_clone = Arc::new(sierra.clone());
        let path_clone = path.clone();
        let compilation_future =
            tokio::task::spawn_blocking(move || sierra_clone.info.contract_class.compile_to_native(&path_clone));

        match handle.block_on(tokio::time::timeout(timeout, compilation_future)) {
            Ok(Ok(Ok(executor))) => Ok(executor),
            Ok(Ok(Err(e))) => {
                timer.finish(false, false);
                Err(NativeCompilationError::CompilationFailed(format!("{:#}", e)))
            }
            Ok(Err(e)) => {
                timer.finish(false, false);
                Err(NativeCompilationError::CompilationFailed(format!("Task panicked: {:#}", e)))
            }
            Err(_) => {
                timer.finish(false, true);
                let _ = std::fs::remove_file(path);
                Err(NativeCompilationError::CompilationTimeout(timeout))
            }
        }
    } else {
        // Blocking context detected - compilation executed directly with timeout using std::thread
        let sierra_clone = Arc::new(sierra.clone());
        let path_clone = path.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = sierra_clone.info.contract_class.compile_to_native(&path_clone);
            let _ = tx.send(result);
        });

        // Wait with timeout
        match rx.recv_timeout(timeout) {
            Ok(Ok(executor)) => Ok(executor),
            Ok(Err(e)) => {
                timer.finish(false, false);
                Err(NativeCompilationError::CompilationFailed(format!("{:#}", e)))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                timer.finish(false, true);
                let _ = std::fs::remove_file(path);
                Err(NativeCompilationError::CompilationTimeout(timeout))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                timer.finish(false, false);
                Err(NativeCompilationError::CompilationFailed("Compilation thread disconnected".to_string()))
            }
        }
    }
}

/// Cache a successfully compiled native class and enforce disk limits.
///
/// This function handles the post-compilation steps:
/// 1. Creates the native class wrapper
/// 2. Evicts cache if needed
/// 3. Inserts into memory cache
/// 4. Enforces disk cache limits
fn cache_compiled_native_class(
    class_hash: ClassHash,
    executor: AotContractExecutor,
    blockifier_compiled_class: blockifier::execution::contract_class::CompiledClassV1,
    config: &config::NativeConfig,
) -> NativeCompiledClass {
    let native_class = NativeCompiledClass::new(executor, blockifier_compiled_class);

    // Inserted first, then evicted if needed (reduces lock contention)
    cache::cache_insert(class_hash, native_class.clone(), Instant::now());

    // Eviction performed if cache is full (after insert to reduce contention window)
    cache::evict_cache_if_needed(config);

    // Disk cache limit enforced after successful compilation
    let exec_config =
        config.execution_config().expect("finish_compilation should only be called when native execution is enabled");
    if let Err(e) = cache::enforce_disk_cache_limit(&exec_config.cache_dir, exec_config.max_disk_cache_size) {
        tracing::warn!(
            target: "madara_cairo_native",
            cache_dir = %exec_config.cache_dir.display(),
            error = %e,
            "disk_cache_enforcement_failed"
        );
    }

    native_class
}

/// Compile a class synchronously (blocking) and return the result.
///
/// Used when blocking compilation mode is enabled. This function:
/// 1. Validates the class hash
/// 2. Creates the cache directory if needed
/// 3. Compiles the Sierra class to native code using `compile_to_native()`
/// 4. Converts to blockifier compiled class format
/// 5. Caches the result in memory and enforces disk cache limits
///
/// **On failure**: Returns `NativeCompilationError` which propagates upstream
/// and causes transaction failure (no VM fallback in blocking mode).
///
/// Requires config to be passed as a parameter (no global config fallback).
pub(crate) fn compile_native_blocking(
    class_hash: ClassHash,
    sierra: &SierraConvertedClass,
    config: &config::NativeConfig,
) -> Result<NativeCompiledClass, NativeCompilationError> {
    let path = cache::get_native_cache_path(&class_hash, config);

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NativeCompilationError::CacheDirectoryError(format!("Failed to create cache directory {:?}: {}", parent, e))
        })?;
    }

    let start = Instant::now();
    let timer = super::metrics::CompilationTimer::new();

    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        path = %path.display(),
        "compilation_blocking_start"
    );

    // Compilation executed - timer consumed by execute_native_compilation
    let exec_config = config
        .execution_config()
        .expect("compile_native_blocking should only be called when native execution is enabled");
    let executor = match execute_native_compilation(sierra, &path, exec_config.compilation_timeout, timer) {
        Ok(executor) => executor,
        Err(e) => {
            // Timer consumed in execute_native_compilation, metrics already recorded
            return Err(e);
        }
    };

    // Converted to blockifier class using common logic
    let blockifier_compiled_class = convert_sierra_to_blockifier_class(sierra)?;

    let elapsed = start.elapsed();
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        elapsed = ?elapsed,
        elapsed_ms = elapsed.as_millis(),
        "compilation_blocking_success"
    );

    // Compiled class cached
    let arc_native = cache_compiled_native_class(class_hash, executor, blockifier_compiled_class, config);

    // Success recorded with actual compilation duration (timer was consumed in execute_native_compilation)
    let duration_ms = elapsed.as_millis() as u64;
    super::metrics::metrics().record_compilation_end(duration_ms, true, false);
    Ok(arc_native)
}

/// Handle successful async compilation - cache result and update metrics.
fn handle_async_compilation_success(
    class_hash: ClassHash,
    executor: AotContractExecutor,
    sierra: &SierraConvertedClass,
    start: Instant,
    timer: super::metrics::CompilationTimer,
    config: &config::NativeConfig,
) {
    // Convert to blockifier class using common logic
    let blockifier_compiled_class = match convert_sierra_to_blockifier_class(sierra) {
        Ok(compiled) => compiled,
        Err(e) => {
            // Conversion failed - log error and mark for retry
            tracing::error!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                error = %e,
                "compilation_async_conversion_failed"
            );
            timer.finish(false, false);
            // Mark this class as failed (with timestamp for eviction)
            let exec_config = config
                .execution_config()
                .expect("handle_async_compilation_success should only be called when native execution is enabled");
            evict_failed_compilations_if_needed(exec_config.max_failed_compilations);
            FAILED_COMPILATIONS.insert(class_hash, Instant::now());
            return;
        }
    };

    let compile_elapsed = start.elapsed();

    // Use existing cache function to avoid code duplication
    cache_compiled_native_class(class_hash, executor, blockifier_compiled_class, config);

    // Removed from failed compilations if present (successful retry)
    remove_failed_compilation(&class_hash);
    let cache_size = cache::cache_len();

    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        elapsed = ?compile_elapsed,
        cache_size = cache_size,
        "compilation_async_success"
    );

    timer.finish(true, false);
}

/// Handle failed async compilation - log error/warning, record metrics, mark for retry.
fn handle_async_compilation_failure(
    class_hash: ClassHash,
    error_kind: &str,
    error_msg: String,
    path: &PathBuf,
    timer: super::metrics::CompilationTimer,
    is_timeout: bool,
    config: &config::NativeConfig,
) {
    if is_timeout {
        // Timeouts are warnings - compilation can be retried later
        tracing::warn!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            error = %error_msg,
            error_kind = error_kind,
            "compilation_async_timeout"
        );
        // Partial file cleanup attempted
        let _ = std::fs::remove_file(path);
        timer.finish(false, true);
    } else {
        // Other failures are errors
        tracing::error!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            error = %error_msg,
            error_kind = error_kind,
            "compilation_async_failed"
        );
        timer.finish(false, false);
    }

    // Mark this class as failed (with timestamp for eviction)
    let exec_config = config
        .execution_config()
        .expect("handle_async_compilation_failure should only be called when native execution is enabled");
    evict_failed_compilations_if_needed(exec_config.max_failed_compilations);
    mark_failed_compilation(class_hash, Instant::now());
}

/// Spawn a background compilation task if one isn't already running for this class.
///
/// **Ownership**: This is the ONLY function that inserts into `COMPILATION_IN_PROGRESS`.
/// All other functions only read from or remove from it. This ensures clear ownership
/// and prevents race conditions.
///
/// Uses atomic entry API to prevent race conditions when multiple requests arrive simultaneously.
/// The atomic check-and-insert ensures only one compilation task is spawned per class hash.
///
/// **Execution Flow**:
/// 1. Atomically checks and inserts into `COMPILATION_IN_PROGRESS` to prevent duplicates
/// 2. Checks if we're in a Tokio runtime context (required for spawning)
/// 3. Acquires a compilation permit (respects concurrency limits)
/// 4. Gets the existing lock from `COMPILATION_IN_PROGRESS` (already inserted)
/// 5. Spawns compilation in a blocking task with timeout
/// 6. Handles success: caches result, enforces disk limits
/// 7. Handles failure: logs error, records metrics, marks for retry
/// 8. Always removes entry from `COMPILATION_IN_PROGRESS` on completion/failure
///
/// **On failure**: The class is added to `FAILED_COMPILATIONS` for automatic
/// retry on the next request. VM fallback is used for the current request.
pub(crate) fn spawn_compilation_if_needed(
    class_hash: ClassHash,
    sierra: Arc<SierraConvertedClass>,
    config: Arc<config::NativeConfig>,
) {
    // Use mark_compilation_in_progress to atomically check and mark compilation in progress
    // This reuses the atomic entry logic and prevents duplicate compilations
    let lock = match mark_compilation_in_progress(class_hash) {
        Some(lock) => {
            // Successfully marked as in progress - proceed with compilation
            lock
        }
        None => {
            // Already compiling (entry was occupied) - nothing to do
            return;
        }
    };

    // Lock is now held, proceed with compilation setup
    // Note: We don't need to wait on the lock here since this is async compilation
    let _ = lock;

    let exec_config = config
        .execution_config()
        .expect("spawn_compilation_if_needed should only be called when native execution is enabled");
    let in_progress_count = get_current_compilations_count();
    let max_concurrent = exec_config.max_concurrent_compilations;

    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        in_progress = in_progress_count,
        max_concurrent = max_concurrent,
        "compilation_async_spawning"
    );

    // Check if we're in a Tokio runtime context
    // This can be called from blockifier's worker pool threads which don't have a Tokio runtime
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => {
            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                "compilation_async_no_runtime_context"
            );
            remove_compilation_in_progress(&class_hash);
            return;
        }
    };

    let compilation_timeout = exec_config.compilation_timeout;

    // Spawn background task for native compilation on the detected runtime
    handle.spawn(async move {
        // Acquire compilation slot
        let permit = match try_acquire_compilation_permit() {
            Some(permit) => permit,
            None => {
                tracing::warn!(
                    target: "madara_cairo_native",
                    class_hash = %format!("{:#x}", class_hash.to_felt()),
                    "compilation_async_max_concurrent_reached"
                );
                remove_compilation_in_progress(&class_hash);
                return;
            }
        };

        // Get the lock that was already created above
        // The entry should always exist here since it was atomically inserted before spawning this task
        let lock = match get_compilation_lock(&class_hash) {
            Some(lock) => lock,
            None => {
                // Entry was removed (shouldn't happen, but handle gracefully)
                tracing::warn!(
                    target: "madara_cairo_native",
                    class_hash = %format!("{:#x}", class_hash.to_felt()),
                    "compilation_async_entry_missing"
                );
                drop(permit);
                return;
            }
        };
        let _guard = lock.write().await;

        // Cache checked again in case another task compiled it
        if cache::cache_contains(&class_hash) {
            remove_compilation_in_progress(&class_hash);
            drop(permit);
            return;
        }

        let path = cache::get_native_cache_path(&class_hash, &config);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!(
                    target: "madara_cairo_native",
                    cache_dir = %parent.display(),
                    error = %e,
                    "compilation_async_cache_directory_creation_failed"
                );
                remove_compilation_in_progress(&class_hash);
                drop(permit);
                return;
            }
        }

        // Execute async compilation
        let start = Instant::now();
        let timer = super::metrics::CompilationTimer::new();

        tracing::debug!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            path = %path.display(),
            timeout_secs = compilation_timeout.as_secs(),
            "compilation_async_start"
        );

        // Use existing compile_to_native function in a blocking task with timeout
        let sierra_clone = sierra.clone();
        let path_clone = path.clone();
        let compilation_future =
            tokio::task::spawn_blocking(move || sierra_clone.info.contract_class.compile_to_native(&path_clone));

        let compilation_result = tokio::time::timeout(compilation_timeout, compilation_future).await;

        // Only one branch will execute, so moving timer is fine
        match compilation_result {
            Ok(Ok(Ok(executor))) => {
                handle_async_compilation_success(class_hash, executor, &sierra, start, timer, &config);
            }
            Ok(Ok(Err(e))) => {
                handle_async_compilation_failure(
                    class_hash,
                    "failed",
                    format!("{:#}", e),
                    &path,
                    timer,
                    false,
                    &config,
                );
            }
            Ok(Err(e)) => {
                handle_async_compilation_failure(class_hash, "panic", format!("{:#}", e), &path, timer, false, &config);
            }
            Err(_) => {
                handle_async_compilation_failure(
                    class_hash,
                    "timeout",
                    format!("Compilation exceeded timeout of {:?}", compilation_timeout),
                    &path,
                    timer,
                    true,
                    &config,
                );
            }
        }

        remove_compilation_in_progress(&class_hash);
        drop(permit);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Helper to create unique test class hash (module_id=3 for compilation.rs)
    fn create_unique_test_class_hash() -> ClassHash {
        crate::test_utils::create_unique_test_class_hash(3)
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_compilation_semaphore_limit() {
        // Use a simple mutex guard pattern similar to execution.rs tests
        static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = crate::metrics::test_counters::acquire_and_reset();

        // Clear any existing state
        clear_compilations_in_progress();
        cache::cache_clear();
        clear_failed_compilations();

        // Set a low semaphore limit (2 concurrent compilations)
        let max_concurrent = 2;
        init_compilation_semaphore(max_concurrent);

        // Create config with async mode
        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
        let config = std::sync::Arc::new(crate::test_utils::create_test_config(
            &temp_dir,
            Some(config::NativeCompilationMode::Async),
            false,
        ));

        // Create test Sierra class
        let sierra = Arc::new(crate::test_utils::get_test_sierra_class().clone());

        // Spawn 5 compilations (more than the limit of 2)
        let num_compilations = 5;
        let mut class_hashes = Vec::new();

        for _ in 0..num_compilations {
            let class_hash = create_unique_test_class_hash();

            // Clear cache to force compilation
            cache::cache_remove(&class_hash);
            remove_compilation_in_progress(&class_hash);

            class_hashes.push(class_hash);

            // Spawn compilation
            spawn_compilation_if_needed(class_hash, sierra.clone(), config.clone());
        }

        // Wait a bit to allow compilations to start and acquire semaphore permits
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify that at most `max_concurrent` compilations are in progress
        // (The semaphore should limit how many compilations actually start)
        let in_progress_count = get_current_compilations_count();
        assert!(
            in_progress_count <= max_concurrent,
            "Should have at most {} compilations in progress (semaphore limit), got {}",
            max_concurrent,
            in_progress_count
        );

        // Wait for all compilations to complete (with timeout)
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(60);
        while start.elapsed() < timeout {
            if get_current_compilations_count() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Verify all compilations completed
        assert_eq!(get_current_compilations_count(), 0, "All compilations should have completed");

        // Verify config has valid max_concurrent_compilations
        let exec_config = config.execution_config().expect("test should use enabled config");
        assert!(exec_config.max_concurrent_compilations > 0);
    }
}
