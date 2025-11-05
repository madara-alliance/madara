//! Compilation orchestration for Cairo Native
//!
//! This module handles the compilation of Sierra classes to native code,
//! including blocking and async modes, retry logic, and concurrency control.

use blockifier::execution::native::contract_class::NativeCompiledClassV1;
use cairo_native::executor::AotContractExecutor;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::config;
use super::error::NativeCompilationError;
use crate::SierraConvertedClass;

use super::cache;

/// Tracks which classes are currently being compiled to avoid duplicate compilations.
///
/// Uses RwLock entries to coordinate between multiple threads requesting the same class.
/// Lock is released after compilation completes (success or failure).
pub(crate) static COMPILATION_IN_PROGRESS: std::sync::LazyLock<
    DashMap<starknet_types_core::felt::Felt, Arc<RwLock<()>>>,
> = std::sync::LazyLock::new(DashMap::new);

/// Tracks classes that failed compilation in async mode (for retry on next request).
///
/// When a compilation fails in async mode, the class hash is added here.
/// On the next request for the same class, compilation is automatically retried.
/// Successful compilation removes the entry from this set.
pub(crate) static FAILED_COMPILATIONS: std::sync::LazyLock<DashMap<starknet_types_core::felt::Felt, ()>> =
    std::sync::LazyLock::new(DashMap::new);

/// Semaphore to limit concurrent compilations.
///
/// Initialized with the actual config value during `init_config()`.
/// Uses `OnceLock` so it can be set once at startup with the correct value.
/// If accessed before initialization, falls back to reading config and creating a temporary semaphore.
static COMPILATION_SEMAPHORE: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();

/// Initialize the compilation semaphore with the actual config value.
///
/// This must be called during config initialization (via `init_config()`).
/// The semaphore limit controls how many compilations can run concurrently.
pub fn init_compilation_semaphore(max_concurrent: usize) {
    COMPILATION_SEMAPHORE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent)));
}

/// Try to acquire a compilation permit, respecting the current config limit.
///
/// Returns `None` if the semaphore limit has been reached.
/// If the semaphore hasn't been initialized yet, reads from config and creates a temporary one.
fn try_acquire_compilation_permit() -> Option<tokio::sync::SemaphorePermit<'static>> {
    // Get semaphore, initializing if needed (should be initialized by init_config, but handle fallback)
    let semaphore = COMPILATION_SEMAPHORE.get_or_init(|| {
        // Fallback: semaphore not initialized yet, use current config value
        let max_concurrent = config::get_max_concurrent_compilations();
        Arc::new(tokio::sync::Semaphore::new(max_concurrent))
    });

    semaphore.try_acquire().ok()
}

/// Validate that a class hash is valid for use in native compilation.
///
/// Ensures:
/// - Class hash is non-zero (zero is not a valid class hash in Starknet)
/// - Hex representation length is within safe limits for filename usage
pub(crate) fn validate_class_hash(class_hash: &starknet_types_core::felt::Felt) -> Result<(), NativeCompilationError> {
    // Validate that class hash is non-zero (zero is not a valid class hash in Starknet)
    if *class_hash == starknet_types_core::felt::Felt::ZERO {
        return Err(NativeCompilationError::InvalidClassHash(*class_hash, "Class hash cannot be zero".to_string()));
    }

    // Additional check: ensure the hex representation is valid for filename
    let hash_str = format!("{:#x}", class_hash);
    if hash_str.is_empty() || hash_str.len() > config::MAX_CLASS_HASH_HEX_LENGTH {
        return Err(NativeCompilationError::InvalidClassHash(
            *class_hash,
            format!("Invalid class hash length: {}", hash_str.len()),
        ));
    }
    Ok(())
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
    class_hash: starknet_types_core::felt::Felt,
    executor: AotContractExecutor,
    blockifier_compiled_class: blockifier::execution::contract_class::CompiledClassV1,
    override_config: Option<&config::NativeConfig>,
) -> Arc<NativeCompiledClassV1> {
    let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

    let arc_native = Arc::new(native_class);
    // Inserted first, then evicted if needed (reduces lock contention)
    cache::NATIVE_CACHE.insert(class_hash, (arc_native.clone(), Instant::now()));

    // Eviction performed if cache is full (after insert to reduce contention window)
    cache::evict_cache_if_needed();

    // Disk cache limit enforced after successful compilation
    let config = override_config.unwrap_or_else(|| config::get_config());
    if let Err(e) = cache::enforce_disk_cache_limit(&config.cache_dir, config.max_disk_cache_size) {
        tracing::warn!(
            target: "madara.cairo_native",
            cache_dir = %config.cache_dir.display(),
            error = %e,
            "disk_cache_enforcement_failed"
        );
    }

    arc_native
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
/// If `override_config` is provided, it will be used instead of the global config.
/// This is useful for testing with isolated configurations.
pub(crate) fn compile_native_blocking(
    class_hash: starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
    override_config: Option<&config::NativeConfig>,
) -> Result<Arc<NativeCompiledClassV1>, NativeCompilationError> {
    let config = override_config.unwrap_or_else(|| config::get_config());

    // Validate class hash
    validate_class_hash(&class_hash)?;

    let path = cache::get_native_cache_path(&class_hash, Some(config));

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NativeCompilationError::CacheDirectoryError(format!("Failed to create cache directory {:?}: {}", parent, e))
        })?;
    }

    let start = Instant::now();
    let timer = super::metrics::CompilationTimer::new();

    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        path = %path.display(),
        "compilation_blocking_start"
    );

    // Compilation executed - timer consumed by execute_native_compilation
    let executor = match execute_native_compilation(sierra, &path, config.compilation_timeout, timer) {
        Ok(executor) => executor,
        Err(e) => {
            // Timer consumed in execute_native_compilation, metrics already recorded
            return Err(e);
        }
    };

    // Converted to blockifier class using common logic
    let blockifier_compiled_class = convert_sierra_to_blockifier_class(sierra)?;

    let elapsed = start.elapsed();
    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        elapsed = ?elapsed,
        "compilation_blocking_success"
    );

    // Compiled class cached
    let arc_native = cache_compiled_native_class(class_hash, executor, blockifier_compiled_class, override_config);

    // Success recorded with actual compilation duration (timer was consumed in execute_native_compilation)
    let duration_ms = elapsed.as_millis() as u64;
    super::metrics::metrics().record_compilation_end(duration_ms, true, false);
    Ok(arc_native)
}

/// Handle successful async compilation - cache result and update metrics.
fn handle_async_compilation_success(
    class_hash: starknet_types_core::felt::Felt,
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
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                "compilation_async_conversion_failed"
            );
            timer.finish(false, false);
            // Mark this class as failed so we retry on next request
            FAILED_COMPILATIONS.insert(class_hash, ());
            return;
        }
    };

    let compile_elapsed = start.elapsed();

    let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

    // Insert first, then evict if needed (reduces lock contention)
    cache::NATIVE_CACHE.insert(class_hash, (Arc::new(native_class), Instant::now()));

    // Evict if cache is full (after insert to reduce contention window)
    cache::evict_cache_if_needed();

    // Removed from failed compilations if present (successful retry)
    FAILED_COMPILATIONS.remove(&class_hash);
    let cache_size = cache::NATIVE_CACHE.len();

    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        elapsed = ?compile_elapsed,
        cache_size = cache_size,
        "compilation_async_success"
    );

    // Disk cache limit enforced after successful compilation
    if let Err(e) = cache::enforce_disk_cache_limit(&config.cache_dir, config.max_disk_cache_size) {
        tracing::warn!(
            target: "madara.cairo_native",
            cache_dir = %config.cache_dir.display(),
            error = %e,
            "disk_cache_enforcement_failed"
        );
    }

    timer.finish(true, false);
}

/// Handle failed async compilation - log error/warning, record metrics, mark for retry.
fn handle_async_compilation_failure(
    class_hash: starknet_types_core::felt::Felt,
    error_kind: &str,
    error_msg: String,
    path: &PathBuf,
    timer: super::metrics::CompilationTimer,
    is_timeout: bool,
) {
    if is_timeout {
        // Timeouts are warnings - compilation can be retried later
        tracing::warn!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
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
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            error = %error_msg,
            error_kind = error_kind,
            "compilation_async_failed"
        );
        timer.finish(false, false);
    }

    // Mark this class as failed so we retry on next request
    FAILED_COMPILATIONS.insert(class_hash, ());
}

/// Execute async compilation in a background task.
///
/// This function performs the actual compilation work for async mode.
/// It should be called from within a spawned async task with proper locks.
async fn execute_async_compilation(
    class_hash: starknet_types_core::felt::Felt,
    sierra: Arc<SierraConvertedClass>,
    path: PathBuf,
    compilation_timeout: Duration,
    config: config::NativeConfig,
) {
    let start = Instant::now();
    let timer = super::metrics::CompilationTimer::new();

    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
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
            handle_async_compilation_failure(class_hash, "failed", format!("{:#}", e), &path, timer, false);
        }
        Ok(Err(e)) => {
            handle_async_compilation_failure(class_hash, "panic", format!("{:#}", e), &path, timer, false);
        }
        Err(_) => {
            handle_async_compilation_failure(
                class_hash,
                "timeout",
                format!("Compilation exceeded timeout of {:?}", compilation_timeout),
                &path,
                timer,
                true,
            );
        }
    }
}

/// Spawn a background compilation task for a class.
///
/// Used in async mode to compile classes without blocking execution.
/// This function:
/// 1. Checks if we're in a Tokio runtime context (required for spawning)
/// 2. Acquires a compilation permit (respects concurrency limits)
/// 3. Checks for duplicate compilations and waits if needed
/// 4. Spawns compilation in a blocking task with timeout
/// 5. Handles success: caches result, enforces disk limits
/// 6. Handles failure: logs error, records metrics, marks for retry
///
/// **On failure**: The class is added to `FAILED_COMPILATIONS` for automatic
/// retry on the next request. VM fallback is used for the current request.
pub(crate) fn spawn_native_compilation(class_hash: starknet_types_core::felt::Felt, sierra: Arc<SierraConvertedClass>) {
    // Check if we're in a Tokio runtime context
    // This can be called from blockifier's worker pool threads which don't have a Tokio runtime
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => {
            tracing::debug!(
                class_hash = %format!("{:#x}", class_hash),
                "cairo_native.compilation.async.no_runtime_context"
            );
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            return;
        }
    };

    let config = config::get_config().clone();
    let compilation_timeout = config.compilation_timeout;

    // Spawn background task for native compilation on the detected runtime
    handle.spawn(async move {
        // Acquire compilation slot
        let permit = match try_acquire_compilation_permit() {
            Some(permit) => permit,
            None => {
                tracing::warn!(
                    class_hash = %format!("{:#x}", class_hash),
                    "cairo_native.compilation.async.max_concurrent_reached"
                );
                COMPILATION_IN_PROGRESS.remove(&class_hash);
                return;
            }
        };

        let lock = COMPILATION_IN_PROGRESS.entry(class_hash).or_insert_with(|| Arc::new(RwLock::new(()))).clone();
        let _guard = lock.write().await;

        // Cache checked again in case another task compiled it
        if cache::NATIVE_CACHE.contains_key(&class_hash) {
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            drop(permit);
            return;
        }

        // Class hash validated
        if let Err(e) = validate_class_hash(&class_hash) {
            tracing::error!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                "compilation_async_invalid_class_hash"
            );
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            drop(permit);
            return;
        }

        let path = cache::get_native_cache_path(&class_hash, None);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!(
                    target: "madara.cairo_native",
                    cache_dir = %parent.display(),
                    error = %e,
                    "compilation_async_cache_directory_creation_failed"
                );
                COMPILATION_IN_PROGRESS.remove(&class_hash);
                drop(permit);
                return;
            }
        }

        // Compilation executed
        execute_async_compilation(class_hash, sierra, path, compilation_timeout, config).await;

        COMPILATION_IN_PROGRESS.remove(&class_hash);
        drop(permit);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use starknet_types_core::felt::Felt;

    #[test]
    fn test_validate_class_hash_valid() {
        let hash = Felt::from(12345u64);
        assert!(validate_class_hash(&hash).is_ok());
    }

    #[test]
    fn test_validate_class_hash_zero() {
        let hash = Felt::ZERO;
        assert!(validate_class_hash(&hash).is_err());
    }

    #[tokio::test]
    async fn test_compilation_semaphore_limit() {
        let config = crate::native_config::get_config();

        // Test that semaphore can be accessed (it's a OnceLock, so we can't directly test)
        // This test mainly ensures the module structure is correct
        assert!(config.max_concurrent_compilations > 0);
    }
}
