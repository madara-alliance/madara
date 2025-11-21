//! Execution flow for Cairo Native classes
//!
//! This module handles the main execution flow for Sierra classes with native execution support.

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use mp_convert::ToFelt;
use serde::de::Error as _;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use starknet_api::core::ClassHash;
use std::sync::Arc;
use std::time::Instant;

use super::config;
use mp_class::{CompiledSierra, SierraClassInfo, SierraConvertedClass};

use super::cache;
use super::compilation;

/// Handle Sierra class conversion with native execution support.
///
/// Implements the full caching and compilation flow for Sierra classes.
pub(crate) fn handle_sierra_class(
    sierra: &SierraConvertedClass,
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<CompiledSierra>,
    info: &SierraClassInfo,
    config: Arc<config::NativeConfig>,
) -> Result<RunnableCompiledClass, ProgramError> {
    // Convert Felt to ClassHash at the entry point for type safety
    let class_hash_typed = ClassHash(*class_hash);

    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        native_enabled = config.is_enabled(),
        "handle_sierra_class"
    );

    // If native execution is disabled, use VM immediately
    if !config.is_enabled() {
        return handle_native_disabled(class_hash, compiled, info);
    }

    // Native execution enabled - try cache first, then compile if needed
    let start = Instant::now();

    // Check cache (memory then disk)
    if let Some(cached) = try_cache_lookup(&class_hash_typed, sierra, start, &config) {
        return Ok(cached);
    }

    // Handle compilation based on mode (blocking vs async)
    if config.is_blocking_mode() {
        handle_blocking_compilation(&class_hash_typed, sierra, &config, start)
    } else {
        handle_async_compilation(&class_hash_typed, sierra, compiled, info, &config)
    }
}

/// Handles the case when native execution is disabled.
///
/// Converts the Sierra class to VM format and returns it immediately.
fn handle_native_disabled(
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<CompiledSierra>,
    info: &SierraClassInfo,
) -> Result<RunnableCompiledClass, ProgramError> {
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "disabled_fallback_to_vm"
    );
    convert_to_vm_class(class_hash, compiled, info, "sierra_version_error", "vm_conversion_error")
}

/// Attempts to find the class in cache (memory first, then disk).
///
/// Returns `Some(cached_class)` if found, `None` if not cached.
fn try_cache_lookup(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    start: Instant,
    config: &Arc<config::NativeConfig>,
) -> Option<RunnableCompiledClass> {
    // Check in-memory cache first (fastest)
    if let Some(cached) = cache::try_get_from_memory_cache(class_hash, config) {
        return Some(cached);
    }

    // Try disk cache (slower but persistent)
    // The guard clause for compilation in progress is handled inside try_get_from_disk_cache
    match cache::try_get_from_disk_cache(class_hash, sierra, config) {
        Ok(Some(cached)) => Some(cached),
        Ok(None) => {
            // Check if None was returned due to compilation in progress (skip) or genuine miss
            if compilation::is_compilation_in_progress(class_hash) {
                // Disk lookup was skipped due to compilation in progress - don't record as miss
                None
            } else {
                // Disk cache miss - genuine miss (file not found/empty)
                let elapsed = start.elapsed();
                super::metrics::metrics().record_cache_disk_miss();
                tracing::debug!(
                    target: "madara_cairo_native",
                    class_hash = %format!("{:#x}", class_hash.to_felt()),
                    elapsed = ?elapsed,
                    elapsed_ms = elapsed.as_millis(),
                    "cache_miss_both_memory_and_disk"
                );
                None
            }
        }
        Err(e) => {
            // Error loading from disk - log and fall through to compilation
            let elapsed = start.elapsed();
            super::metrics::metrics().record_cache_disk_error_fallback();
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                error = %e,
                elapsed = ?elapsed,
                "disk_cache_error_fallback"
            );
            None
        }
    }
}

/// Waits for compilation to complete by polling the cache.
///
/// This helper function polls the cache at regular intervals until:
/// - The class appears in cache (success)
/// - Compilation fails (error)
/// - Timeout is reached (error)
///
/// Returns the compiled class if found, or an error if compilation failed or timed out.
fn wait_for_compilation_completion(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &Arc<config::NativeConfig>,
    start: Instant,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) -> Result<RunnableCompiledClass, ProgramError> {
    let deadline = start + timeout;

    loop {
        // Check timeout first
        let now = Instant::now();
        if now > deadline {
            let elapsed = now.duration_since(start);
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                elapsed_secs = elapsed.as_secs(),
                timeout_secs = timeout.as_secs(),
                "compilation_blocking_timeout"
            );
            return Err(ProgramError::Parse(serde_json::Error::custom(format!(
                "Compilation timeout after {:?} for class hash {:#x}",
                timeout,
                class_hash.to_felt()
            ))));
        }

        // Check if compilation is still in progress
        // Use get_compilation_lock to check if compilation is in progress (reuses the lock check logic)
        let compilation_lock = compilation::get_compilation_lock(class_hash);
        if compilation_lock.is_none() {
            // Compilation completed - try to get from cache (reuse existing cache lookup logic)
            if let Some(cached) = try_cache_lookup(class_hash, sierra, start, config) {
                let conversion_start = Instant::now();
                let conversion_elapsed = conversion_start.elapsed();
                let total_elapsed = now.duration_since(start);
                super::metrics::metrics().record_compilation_blocking_complete();
                super::metrics::metrics()
                    .record_compilation_blocking_conversion_time(conversion_elapsed.as_millis() as u64);
                tracing::debug!(
                    target: "madara_cairo_native",
                    class_hash = %format!("{:#x}", class_hash.to_felt()),
                    elapsed = ?total_elapsed,
                    elapsed_ms = total_elapsed.as_millis(),
                    conversion_ms = conversion_elapsed.as_millis(),
                    "compilation_blocking_complete"
                );
                return Ok(cached);
            }

            // Compilation finished but not in cache - check if it failed
            if compilation::has_failed_compilation(class_hash) {
                let elapsed = now.duration_since(start);
                tracing::warn!(
                    target: "madara_cairo_native",
                    class_hash = %format!("{:#x}", class_hash.to_felt()),
                    elapsed_secs = elapsed.as_secs(),
                    "compilation_blocking_failed"
                );
                return Err(ProgramError::Parse(serde_json::Error::custom(format!(
                    "Compilation failed for class hash {:#x}",
                    class_hash.to_felt()
                ))));
            }
        }

        // Wait before next poll
        std::thread::sleep(poll_interval);
    }
}

/// Handles compilation in blocking mode.
///
/// In blocking mode, this function waits for compilation to complete rather than falling back to VM.
/// - If compilation is already in progress (from async mode), it waits for it to complete.
/// - Otherwise, it starts compilation synchronously and waits for it to finish.
///
/// **Concurrent Compilation Protection**: Uses atomic entry API to prevent duplicate compilations.
/// This ensures only one compilation happens per class hash.
fn handle_blocking_compilation(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &Arc<config::NativeConfig>,
    start: Instant,
) -> Result<RunnableCompiledClass, ProgramError> {
    use std::time::Duration;

    super::metrics::metrics().record_compilation_blocking_miss();
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        "compilation_blocking_miss"
    );

    // Get timeout and poll interval from config
    let exec_config = config
        .execution_config()
        .expect("handle_blocking_compilation should only be called when native execution is enabled");
    let compilation_timeout = exec_config.compilation_timeout;
    let poll_interval = Duration::from_millis(config::DEFAULT_COMPILATION_POLL_INTERVAL_MS);

    // Check if compilation is already in progress
    if compilation::is_compilation_in_progress(class_hash) {
        // Compilation already in progress - wait for it to complete
        tracing::debug!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            timeout_secs = compilation_timeout.as_secs(),
            poll_interval_ms = poll_interval.as_millis(),
            "compilation_already_in_progress_waiting"
        );
        return wait_for_compilation_completion(class_hash, sierra, config, start, compilation_timeout, poll_interval);
    }

    // No compilation in progress - try to start it using spawn_compilation_if_needed
    // This ensures atomic check and prevents race conditions
    // Note: spawn_compilation_if_needed requires Tokio runtime; if not available, it returns early
    compilation::spawn_compilation_if_needed(*class_hash, Arc::new(sierra.clone()), config.clone());

    // Check if compilation actually started (spawn_compilation_if_needed may return early if no runtime)
    if compilation::is_compilation_in_progress(class_hash) {
        // Compilation started - wait for it to complete
        tracing::debug!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            timeout_secs = compilation_timeout.as_secs(),
            poll_interval_ms = poll_interval.as_millis(),
            "compilation_blocking_waiting"
        );
        return wait_for_compilation_completion(class_hash, sierra, config, start, compilation_timeout, poll_interval);
    }

    // No Tokio runtime available - fall back to synchronous compilation
    // This handles the case where spawn_compilation_if_needed couldn't start async compilation
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        "compilation_blocking_no_runtime_fallback_to_sync"
    );

    // Use synchronous compilation directly
    match compilation::compile_native_blocking(*class_hash, sierra, config.as_ref()) {
        Ok(native_class) => {
            let conversion_start = Instant::now();
            let runnable = native_class.to_runnable();
            let conversion_elapsed = conversion_start.elapsed();
            let total_elapsed = start.elapsed();
            super::metrics::metrics().record_compilation_blocking_complete();
            super::metrics::metrics()
                .record_compilation_blocking_conversion_time(conversion_elapsed.as_millis() as u64);
            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                elapsed = ?total_elapsed,
                elapsed_ms = total_elapsed.as_millis(),
                conversion_ms = conversion_elapsed.as_millis(),
                "compilation_blocking_complete"
            );
            Ok(runnable)
        }
        Err(e) => {
            tracing::warn!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                error = %e,
                "compilation_blocking_failed"
            );
            Err(ProgramError::Parse(serde_json::Error::custom(format!(
                "Compilation failed for class hash {:#x}: {}",
                class_hash.to_felt(),
                e
            ))))
        }
    }
}

/// Handles compilation in async mode.
///
/// Spawns background compilation and immediately falls back to VM execution.
///
/// **Retry Logic**: If a class previously failed compilation (recorded in `FAILED_COMPILATIONS`),
/// compilation is automatically retried on the next request **only if** `config.enable_retry` is `true`.
/// When retry is enabled and a retry occurs, a log message is emitted.
///
/// Retrying is beneficial when:
/// - Previous compilation failed due to transient issues (timeout, temporary resource constraints)
/// - System conditions have improved (more resources available, less load)
/// - The failure was due to a race condition or temporary file system issue
fn handle_async_compilation(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    compiled: &Arc<CompiledSierra>,
    info: &SierraClassInfo,
    config: &Arc<config::NativeConfig>,
) -> Result<RunnableCompiledClass, ProgramError> {
    // Check if this class previously failed compilation
    let is_failed = compilation::has_failed_compilation(class_hash);

    // Handle retry logic based on config
    let exec_config = config
        .execution_config()
        .expect("handle_async_compilation should only be called when native execution is enabled");
    if is_failed {
        if exec_config.enable_retry {
            // Retry enabled: remove from failed list and retry compilation
            compilation::remove_failed_compilation(class_hash);
            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                "compilation_retrying_previously_failed"
            );
            // Spawn compilation to retry
            spawn_compilation_if_needed(class_hash, sierra, config);
        } else {
            // Retry disabled: keep in failed list to prevent retrying
            // Don't spawn compilation - class stays in FAILED_COMPILATIONS
            tracing::debug!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                "compilation_skipped_previously_failed_retry_disabled"
            );
        }
    } else {
        // Not a failed class - spawn compilation normally for new requests
        spawn_compilation_if_needed(class_hash, sierra, config);
    }

    // Fall back to VM while compilation happens in background
    super::metrics::metrics().record_vm_fallback();
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        "fallback_to_vm_async"
    );
    // Convert ClassHash back to Felt for VM conversion (which uses Felt internally)
    let felt = class_hash.to_felt();
    convert_to_vm_class(&felt, compiled, info, "async_sierra_version_error", "async_vm_conversion_error")
}

/// Spawns a background compilation task if one isn't already running for this class.
///
/// Uses atomic entry API to prevent race conditions when multiple requests arrive simultaneously.
/// All COMPILATION_IN_PROGRESS management is handled by the compilation module.
fn spawn_compilation_if_needed(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &Arc<config::NativeConfig>,
) {
    // Delegate to compilation module - it owns COMPILATION_IN_PROGRESS
    compilation::spawn_compilation_if_needed(*class_hash, Arc::new(sierra.clone()), config.clone());
}

/// Converts a CompiledSierra to RunnableCompiledClass for VM execution.
///
/// This is used when native execution is disabled or as a fallback during async compilation.
fn convert_to_vm_class(
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<CompiledSierra>,
    info: &SierraClassInfo,
    sierra_version_error_label: &str,
    vm_conversion_error_label: &str,
) -> Result<RunnableCompiledClass, ProgramError> {
    let sierra_version = info.contract_class.sierra_version().map_err(|e| {
        tracing::error!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            error = %e,
            "{}", sierra_version_error_label
        );
        ProgramError::Parse(serde::de::Error::custom("Failed to get sierra version from program"))
    })?;

    let vm_class = RunnableCompiledClass::try_from(ApiContractClass::V1((
        compiled.as_ref().try_into().map_err(|e| {
            tracing::error!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %format!("{:?}", e),
                "{}", vm_conversion_error_label
            );
            ProgramError::Parse(serde::de::Error::custom(format!("Failed to convert to VM class: {:?}", e)))
        })?,
        sierra_version,
    )))?;

    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "returning_vm_class"
    );
    Ok(vm_class)
}
#[cfg(test)]
mod tests {
    use super::*;

    // Import test utilities from test_utils
    // Note: assert_counters is a macro exported at crate root due to #[macro_export]
    use crate::assert_counters;
    use crate::test_utils::{assert_is_native_class, assert_is_vm_class};

    use crate::metrics::test_counters;
    use mp_class::SierraConvertedClass;
    use rstest::rstest;
    use starknet_api::core::ClassHash;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;
    // Import fixtures from test_utils
    use crate::test_utils::{async_config, blocking_config, sierra_class, temp_dir};

    // Test mutex to serialize test execution for tests that need to clear/modify caches
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    // Helper function to create a unique test class hash (module_id=2 for execution.rs)
    fn create_unique_test_class_hash() -> ClassHash {
        crate::test_utils::create_unique_test_class_hash(2)
    }

    // Helper function to create a native class, optionally cache it in memory, and optionally verify it exists on disk
    fn create_native_class(
        sierra: &SierraConvertedClass,
        temp_dir: &TempDir,
        class_hash: ClassHash,
        cache_in_memory: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::time::Instant;

        // Create path for compiled .so file
        let so_path = temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()));

        // Use shared function to create native class (this compiles and saves to disk)
        let native_class = crate::test_utils::create_native_class_internal(sierra, &so_path)?;

        // Optionally insert into memory cache
        if cache_in_memory {
            cache::cache_insert(class_hash, native_class, Instant::now());
        }

        // Verify file exists on disk
        assert!(so_path.exists(), "Native class file should exist on disk");

        Ok(())
    }

    // Helper function to poll for compilation completion with timeout (async version)
    // Returns true if compilation completed and class is cached, false if timeout
    async fn wait_for_compilation_completion_async(class_hash: &ClassHash, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let check_interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            // Check if compilation completed and class is in cache
            if cache::cache_contains(class_hash) {
                // Give it a small delay to ensure it's fully cached
                tokio::time::sleep(Duration::from_millis(50)).await;
                return true;
            }

            // Check if compilation is still in progress
            if !compilation::is_compilation_in_progress(class_hash) {
                // Compilation finished but not cached - might have failed
                // Wait a bit more to see if it gets cached
                tokio::time::sleep(check_interval).await;
                if cache::cache_contains(class_hash) {
                    return true;
                }
                // Compilation finished but not cached - likely failed
                return false;
            }

            tokio::time::sleep(check_interval).await;
        }

        // Timeout reached
        false
    }

    #[rstest]
    fn test_handle_sierra_class_memory_cache_hit(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Create config using the temp_dir fixture (so cache_dir matches)
        let async_config =
            crate::test_utils::create_test_config_arc(&temp_dir, Some(config::NativeCompilationMode::Async), true);

        // Pre-populate memory cache
        create_native_class(&sierra_class, &temp_dir, class_hash, true)
            .expect("Failed to create and cache native class");

        // Verify it's in cache before calling handle_sierra_class
        assert!(cache::cache_contains(&class_hash), "Class should be in memory cache before call");

        // Memory cache hit expected
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            async_config.clone(),
        );

        assert!(result.is_ok(), "Should successfully retrieve from memory cache");
        let runnable = result.unwrap();

        // Assert that the returned class IS Native (from memory cache)
        assert_is_native_class(&runnable);

        // Verify it's still in cache after call
        assert!(cache::cache_contains(&class_hash), "Class should remain in memory cache after call");

        // Metrics assertions - memory cache hit, no disk access, no compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 1,
            CACHE_MEMORY_MISS: 0,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 0,
            COMPILATIONS_STARTED: 0,
            VM_FALLBACKS: 0,
        );
    }

    #[rstest]
    fn test_handle_sierra_class_memory_miss_disk_hit(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Create config using the temp_dir fixture (so cache_dir matches)
        let async_config =
            crate::test_utils::create_test_config_arc(&temp_dir, Some(config::NativeCompilationMode::Async), true);

        // Create and save native class to disk using the config's cache directory
        let so_path = cache::get_native_cache_path(&class_hash, &async_config);
        let _native_class = crate::test_utils::create_native_class_internal(&sierra_class, &so_path)
            .expect("Failed to create native class");
        assert!(so_path.exists(), "Native class file should exist on disk");

        // Verify memory cache still doesn't have it
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache before disk load");

        // Disk cache hit expected
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            async_config.clone(),
        );

        assert!(result.is_ok(), "Should successfully retrieve from disk cache");
        let runnable = result.unwrap();

        // Assert that the returned class IS Native (from disk cache)
        assert_is_native_class(&runnable);

        // Verify it was loaded into memory cache after disk hit
        assert!(cache::cache_contains(&class_hash), "Should be loaded into memory cache after disk hit");

        // Metrics assertions - disk cache hit, no compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 1,
            CACHE_DISK_MISS: 0,
            COMPILATIONS_STARTED: 0,
            VM_FALLBACKS: 0,
        );
    }

    #[rstest]
    fn test_handle_sierra_class_native_disabled(sierra_class: SierraConvertedClass, _temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();

        // Create config with native disabled
        let config = Arc::new(config::NativeConfig::Disabled);

        let _metrics_guard = test_counters::acquire_and_reset();

        // VM class returned immediately when native is disabled
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        assert!(result.is_ok(), "Should return VM class when native is disabled");
        let runnable = result.unwrap();

        // Assert that the returned class IS VM (native is disabled)
        assert_is_vm_class(&runnable);

        // Metrics assertions - native disabled
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 0,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 0,
            COMPILATIONS_STARTED: 0,
            VM_FALLBACKS: 0,
        );
    }

    #[rstest]
    fn test_handle_sierra_class_cache_miss_blocking_mode(
        sierra_class: SierraConvertedClass,
        _temp_dir: TempDir,
        blocking_config: Arc<config::NativeConfig>,
    ) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();

        // Ensure our specific class_hash is not in any caches (cleanup from any previous runs)
        cache::cache_remove(&class_hash);
        compilation::remove_compilation_in_progress(&class_hash);
        compilation::remove_failed_compilation(&class_hash);

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );

        // Reset metrics RIGHT BEFORE the operation to minimize window for other tests to interfere
        // This ensures metrics from cache tests running in parallel don't leak in
        let _metrics_guard = test_counters::acquire_and_reset();

        // Blocking mode compiles synchronously
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            blocking_config.clone(),
        );

        assert!(result.is_ok(), "Should successfully compile in blocking mode");
        let runnable = result.unwrap();

        // Assert that the returned class IS Cairo Native (not VM)
        assert_is_native_class(&runnable);
        // Compilation should be complete, so not in progress
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Metrics assertions - cache miss and blocking compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 1,
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
            COMPILATION_BLOCKING_COMPLETE: 1,
            VM_FALLBACKS: 0,
        );
    }

    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_handle_sierra_class_cache_miss_async_mode(
        sierra_class: SierraConvertedClass,
        _temp_dir: TempDir,
        async_config: Arc<config::NativeConfig>,
    ) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should not be in failed_compilations initially"
        );

        let _metrics_guard = test_counters::acquire_and_reset();

        // Async mode returns VM immediately while compilation happens in background
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            async_config.clone(),
        );

        assert!(result.is_ok(), "Should return VM class immediately in async mode");
        let runnable = result.unwrap();

        // Assert that the returned class IS VM (not Native) in async mode
        assert_is_vm_class(&runnable);

        // Wait for compilation completion with 20 second timeout
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation completed and class is cached
        assert!(cache::cache_contains(&class_hash), "Class should be in memory cache after compilation completes");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should not be in failed_compilations after successful compilation"
        );

        // Metrics assertions - cache miss and async compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 1,
            VM_FALLBACKS: 1,
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
        );
    }

    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_handle_sierra_class_disk_cache_error(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Create config using the temp_dir fixture (so cache_dir matches)
        let async_config =
            crate::test_utils::create_test_config_arc(&temp_dir, Some(config::NativeCompilationMode::Async), true);

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Create a corrupt/invalid .so file on disk to trigger disk cache error
        // Use the config's cache directory path
        let so_path = cache::get_native_cache_path(&class_hash, &async_config);
        std::fs::write(&so_path, b"invalid binary data").expect("Failed to write invalid file");

        // Disk error handled gracefully, falls back to compilation
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            async_config.clone(),
        );

        // In async mode, should return VM class even if disk cache fails
        assert!(result.is_ok(), "Should handle disk cache error gracefully");
        let runnable = result.unwrap();
        assert_is_vm_class(&runnable); // Should return VM class in async mode when compilation in progress

        // Wait for compilation completion with 20 second timeout
        // Compilation spawned in background after disk error
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds after disk cache error");

        // Compilation completed successfully despite disk error - verify it's in cache
        assert!(cache::cache_contains(&class_hash), "Class should be in memory cache after compilation completes");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should not be in failed_compilations after successful compilation"
        );

        // Metrics assertions - disk error and fallback to compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 0,
            CACHE_DISK_LOAD_ERROR: 1,
            VM_FALLBACKS: 1,
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
        );
    }

    #[rstest]
    fn test_handle_sierra_class_compilation_in_progress_skip_disk(
        sierra_class: SierraConvertedClass,
        _temp_dir: TempDir,
        async_config: Arc<config::NativeConfig>,
    ) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Mark compilation as in progress (for test setup)
        compilation::insert_compilation_in_progress(class_hash);
        assert!(compilation::is_compilation_in_progress(&class_hash), "Class should be in compilation_in_progress");

        // Disk cache skipped when compilation is in progress
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            async_config.clone(),
        );

        assert!(result.is_ok(), "Should return VM class when compilation is in progress");
        let runnable = result.unwrap();
        assert_is_vm_class(&runnable); // Verify it's VM class, not Native

        // Metrics assertions - compilation in progress, disk skipped
        assert_counters!(
            COMPILATION_IN_PROGRESS_SKIP: 1,
            VM_FALLBACKS: 1,
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 0,
            COMPILATIONS_STARTED: 0,
        );

        // Clean up
        compilation::remove_compilation_in_progress(&class_hash);
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should be removed from compilation_in_progress"
        );
    }

    #[rstest]
    fn test_handle_sierra_class_blocking_compilation_failure(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Create config with very short timeout to force compilation failure
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Blocking)
                .with_compilation_timeout(Duration::from_millis(1)) // Very short timeout
                .build(),
        );

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should not be in failed_compilations initially"
        );

        // Compilation timeout expected in blocking mode with very short timeout
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // In blocking mode, compilation failures/timeouts should return an error (not fall back to VM)
        // This is the key difference from async mode - blocking mode waits and fails if compilation fails
        assert!(
            result.is_err(),
            "Blocking mode should return error on compilation timeout/failure, not fall back to VM"
        );
        let error = result.unwrap_err();
        // Verify error message contains timeout or compilation failure indication
        let error_msg = format!("{:?}", error);
        assert!(
            error_msg.contains("timeout") || error_msg.contains("Compilation") || error_msg.contains("failed"),
            "Error message should indicate compilation timeout or failure: {}",
            error_msg
        );

        // Expected: compilation timeout/failure in blocking mode
        // Verify class is not cached after failure
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache after compilation failure");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after failure"
        );
        // In blocking mode, failures don't go to FAILED_COMPILATIONS (that's async mode only)

        // Metrics assertions - compilation timeout/failure
        assert_counters!(
            CACHE_MEMORY_MISS: 1,
            CACHE_DISK_MISS: 1,
            COMPILATIONS_STARTED: 1,
            VM_FALLBACKS: 0, // No VM fallback in blocking mode
        );
        use std::sync::atomic::Ordering;
        let timeout_or_failed = test_counters::COMPILATIONS_TIMEOUT.load(Ordering::Relaxed)
            + test_counters::COMPILATIONS_FAILED.load(Ordering::Relaxed);
        assert_eq!(timeout_or_failed, 1, "Should have exactly one compilation timeout or failure");
    }

    /// Tests the memory cache timeout scenario.
    ///
    /// Verifies that when a memory cache lookup times out, the system correctly
    /// falls back to disk cache. The timeout duration is configurable via
    /// `native_memory_cache_timeout_ms` in the config.
    ///
    /// Uses an extremely short timeout (1 nanosecond) to reliably trigger the timeout,
    /// since conversion operations take significantly longer. The test validates:
    /// - Timeout configuration is respected
    /// - Fallback to disk cache occurs after memory cache timeout
    /// - Timeout metric (`CACHE_MEMORY_TIMEOUT`) is recorded
    /// - Timeout events are logged appropriately
    #[rstest]
    fn test_handle_sierra_class_memory_cache_timeout(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Configure an extremely short memory cache timeout to reliably trigger timeout
        // 1 nanosecond timeout ensures timeout occurs since conversion operations take longer
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_memory_cache_timeout(Duration::from_nanos(1)) // Extremely short timeout to force timeout
                .build(),
        );

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Pre-populate disk cache to enable testing disk fallback after memory timeout
        create_native_class(&sierra_class, &temp_dir, class_hash, false)
            .expect("Failed to create and save native class to disk");

        // Memory cache lookup will timeout, triggering fallback to disk cache
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        assert!(result.is_ok(), "Should successfully retrieve from disk cache after memory miss");
        let runnable = result.unwrap();

        // Assert that the returned class IS Native (from disk cache)
        assert_is_native_class(&runnable);

        // Verify it was loaded into memory cache after disk hit
        assert!(cache::cache_contains(&class_hash), "Should be loaded into memory cache after disk hit");

        // Metrics assertions - memory timeout, disk hit, no compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_MEMORY_TIMEOUT: 1,
            CACHE_HITS_DISK: 1,
            CACHE_DISK_MISS: 0,
            COMPILATIONS_STARTED: 0,
            VM_FALLBACKS: 0,
        );
    }

    /// Tests the disk cache timeout scenario.
    ///
    /// Verifies that when a disk cache load operation times out, the system correctly
    /// falls back to compilation. The timeout duration is configurable via
    /// `native_disk_cache_load_timeout_secs` in the config.
    ///
    /// Uses an extremely short timeout (1 microsecond) to reliably trigger the timeout,
    /// since loading .so files takes significantly longer. The test validates:
    /// - Timeout configuration is respected
    /// - Fallback to compilation occurs after disk cache timeout
    /// - Timeout metric (`CACHE_DISK_LOAD_TIMEOUT`) is recorded
    /// - Timeout events are logged appropriately
    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_handle_sierra_class_disk_cache_timeout(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Configure an extremely short disk cache timeout to reliably trigger timeout
        // 1 microsecond timeout ensures timeout occurs since disk I/O operations take longer
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_disk_cache_load_timeout(Duration::from_micros(1)) // Extremely short timeout to force timeout
                .build(),
        );

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Pre-populate disk cache to enable testing timeout during disk load operation
        // With 1 microsecond timeout, disk load will timeout since loading .so files takes longer
        create_native_class(&sierra_class, &temp_dir, class_hash, false)
            .expect("Failed to create and save native class to disk");

        // Memory cache misses, disk cache load times out (file exists but load operation times out),
        // triggering fallback to compilation
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // Async mode returns VM immediately while compilation happens in background
        assert!(result.is_ok(), "Should return VM class immediately in async mode");
        let runnable = result.unwrap();

        // Assert that the returned class IS VM (not Native) in async mode
        assert_is_vm_class(&runnable);

        // Wait for compilation completion with 20 second timeout
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation completed and class is cached
        assert!(cache::cache_contains(&class_hash), "Class should be in memory cache after compilation completes");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Metrics assertions - disk timeout and fallback to compilation
        assert_counters!(
            CACHE_HITS_MEMORY: 0,
            CACHE_MEMORY_MISS: 1,
            CACHE_HITS_DISK: 0,
            CACHE_DISK_MISS: 1,
            CACHE_DISK_LOAD_TIMEOUT: 1,
            VM_FALLBACKS: 1,
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
        );
    }

    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_compilation_in_progress_prevents_duplicate(
        sierra_class: SierraConvertedClass,
        _temp_dir: TempDir,
        async_config: Arc<config::NativeConfig>,
    ) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );

        // Spawn multiple concurrent requests for the same class
        let num_concurrent_requests = 10;
        let mut handles = vec![];

        for _ in 0..num_concurrent_requests {
            let sierra_clone = sierra_class.clone();
            let class_hash_clone = class_hash;
            let config_clone = async_config.clone();
            let handle = tokio::spawn(async move {
                handle_sierra_class(
                    &sierra_clone,
                    &class_hash_clone.to_felt(),
                    &sierra_clone.compiled,
                    &sierra_clone.info,
                    config_clone,
                )
            });
            handles.push(handle);
        }

        // Wait a short time to allow all requests to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify only one compilation was started
        assert_counters!(
            COMPILATIONS_STARTED: 1,
        );

        // Verify only one entry in COMPILATION_IN_PROGRESS
        assert_eq!(compilation::get_current_compilations_count(), 1, "Should have exactly one compilation in progress");
        assert!(compilation::is_compilation_in_progress(&class_hash), "Class should be in compilation_in_progress");

        // Wait for compilation to complete
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;
        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Wait for all requests to complete
        let mut results = vec![];
        for handle in handles {
            let result = handle.await.expect("Task should not panic");
            results.push(result);
        }

        // Verify all requests succeeded
        assert_eq!(results.len(), num_concurrent_requests, "All requests should complete");
        for result in &results {
            assert!(result.is_ok(), "All requests should succeed");
        }

        // Verify compilation completed successfully
        assert!(cache::cache_contains(&class_hash), "Class should be in memory cache after compilation completes");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Verify metrics - only one compilation started and succeeded
        assert_counters!(
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
            VM_FALLBACKS: num_concurrent_requests as u64,
        );
    }

    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_async_mode_retry_enabled(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_compilation_timeout(Duration::from_secs(30))
                .with_enable_retry(true)
                .build(),
        );
        let exec_config = config.execution_config().expect("test should use enabled config");
        compilation::init_compilation_semaphore(exec_config.max_concurrent_compilations);

        // Clear any existing state
        cache::cache_remove(&class_hash);
        compilation::remove_compilation_in_progress(&class_hash);
        compilation::remove_failed_compilation(&class_hash);

        // Simulate a previously failed compilation by adding to FAILED_COMPILATIONS
        compilation::mark_failed_compilation(class_hash, Instant::now());
        assert!(compilation::has_failed_compilation(&class_hash), "Class should be in FAILED_COMPILATIONS initially");

        // Request the class with retry enabled
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // Should return VM version immediately (async mode)
        assert!(result.is_ok(), "Should return VM class immediately in async mode");

        // Wait a short time to allow compilation to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify retry happened: class should be removed from FAILED_COMPILATIONS
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should be removed from FAILED_COMPILATIONS when retry is enabled"
        );

        // Verify compilation was spawned (should be in COMPILATION_IN_PROGRESS)
        assert!(
            compilation::is_compilation_in_progress(&class_hash),
            "Compilation should be spawned when retry is enabled"
        );

        // Wait for compilation to complete
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;
        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation succeeded
        assert!(
            cache::cache_contains(&class_hash),
            "Class should be in memory cache after successful retry compilation"
        );
        assert!(
            !compilation::has_failed_compilation(&class_hash),
            "Class should not be in FAILED_COMPILATIONS after successful compilation"
        );

        // Verify metrics
        assert_counters!(
            COMPILATIONS_STARTED: 1,
            COMPILATIONS_SUCCEEDED: 1,
        );
    }

    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_async_mode_retry_disabled(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_compilation_timeout(Duration::from_secs(30))
                .with_enable_retry(false)
                .build(),
        );
        let exec_config = config.execution_config().expect("test should use enabled config");
        compilation::init_compilation_semaphore(exec_config.max_concurrent_compilations);

        // Clear any existing state
        cache::cache_remove(&class_hash);
        compilation::remove_compilation_in_progress(&class_hash);
        compilation::remove_failed_compilation(&class_hash);

        // Simulate a previously failed compilation by adding to FAILED_COMPILATIONS
        compilation::mark_failed_compilation(class_hash, Instant::now());
        assert!(compilation::has_failed_compilation(&class_hash), "Class should be in FAILED_COMPILATIONS initially");

        // Request the class with retry disabled
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // Should return VM version immediately (async mode)
        assert!(result.is_ok(), "Should return VM class immediately in async mode");

        // Wait a short time to ensure no compilation was spawned
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify retry did NOT happen: class should still be in FAILED_COMPILATIONS
        assert!(
            compilation::has_failed_compilation(&class_hash),
            "Class should remain in FAILED_COMPILATIONS when retry is disabled"
        );

        // Verify compilation was NOT spawned (should NOT be in COMPILATION_IN_PROGRESS)
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Compilation should NOT be spawned when retry is disabled"
        );

        // Verify class is NOT in cache (no compilation happened)
        assert!(!cache::cache_contains(&class_hash), "Class should NOT be in memory cache when retry is disabled");

        // Verify metrics - no compilation should have started
        assert_counters!(
            COMPILATIONS_STARTED: 0,
            VM_FALLBACKS: 1,
        );
    }

    #[rstest]
    fn test_compilation_timeout_config(sierra_class: SierraConvertedClass, temp_dir: TempDir) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Clear any existing state
        cache::cache_remove(&class_hash);
        compilation::remove_compilation_in_progress(&class_hash);
        compilation::remove_failed_compilation(&class_hash);

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");

        // Create config with a very short timeout to verify timeout behavior
        // In a real scenario, compilations usually complete quickly, so we use 1ms to force timeout
        let configured_timeout = Duration::from_millis(1);
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Blocking)
                .with_compilation_timeout(configured_timeout)
                .build(),
        );

        // Verify the config has the correct timeout value
        let exec_config = config.execution_config().expect("test should use enabled config");
        assert_eq!(
            exec_config.compilation_timeout, configured_timeout,
            "Config should have the configured timeout value of {:?}",
            configured_timeout
        );

        // Attempt compilation with very short timeout - should timeout
        // In blocking mode, timeouts should return an error (not fall back to VM)
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // In blocking mode, compilation timeouts should return an error (not fall back to VM)
        // This is the key difference from async mode - blocking mode waits and fails if compilation times out
        assert!(result.is_err(), "Blocking mode should return error on compilation timeout, not fall back to VM");
        let error = result.unwrap_err();
        // Verify error message contains timeout indication
        let error_msg = format!("{:?}", error);
        assert!(
            error_msg.contains("timeout") || error_msg.contains("Timeout"),
            "Error message should indicate compilation timeout: {}",
            error_msg
        );

        // Verify timeout metrics were recorded (no VM fallback in blocking mode)
        assert_counters!(
            COMPILATIONS_TIMEOUT: 1,
            VM_FALLBACKS: 0, // No VM fallback in blocking mode
        );
    }

    /// Tests that async mode falls back to VM on compilation timeout/failure.
    ///
    /// This test verifies the key difference between async and blocking modes:
    /// - Async mode: Falls back to VM when compilation fails or times out
    /// - Blocking mode: Returns error when compilation fails or times out
    #[rstest]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_handle_sierra_class_async_compilation_failure_fallback_to_vm(
        sierra_class: SierraConvertedClass,
        temp_dir: TempDir,
    ) {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();

        // Clear any existing state
        cache::cache_remove(&class_hash);
        compilation::remove_compilation_in_progress(&class_hash);
        compilation::remove_failed_compilation(&class_hash);

        // Create config with very short timeout to force compilation failure
        let config = Arc::new(
            config::NativeConfig::builder()
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_compilation_timeout(Duration::from_millis(1)) // Very short timeout
                .build(),
        );

        // Verify caches don't have our unique class_hash
        assert!(!cache::cache_contains(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::is_compilation_in_progress(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );

        // In async mode, compilation timeout/failure should fall back to VM (not return error)
        let result = handle_sierra_class(
            &sierra_class,
            &class_hash.to_felt(),
            &sierra_class.compiled,
            &sierra_class.info,
            config.clone(),
        );

        // Async mode should return VM class immediately (fallback), not error
        assert!(result.is_ok(), "Async mode should fall back to VM on compilation timeout/failure, not return error");
        let runnable = result.unwrap();
        assert_is_vm_class(&runnable); // Verify it's VM class, not Native

        // Wait a bit for compilation to complete/fail in background
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify compilation was attempted and failed/timed out
        assert!(
            compilation::has_failed_compilation(&class_hash),
            "Class should be marked as failed compilation in async mode"
        );

        // Metrics assertions - async mode falls back to VM
        assert_counters!(
            CACHE_MEMORY_MISS: 1,
            CACHE_DISK_MISS: 1,
            COMPILATIONS_STARTED: 1,
            VM_FALLBACKS: 1, // VM fallback occurred in async mode
        );
        use std::sync::atomic::Ordering;
        let timeout_or_failed = test_counters::COMPILATIONS_TIMEOUT.load(Ordering::Relaxed)
            + test_counters::COMPILATIONS_FAILED.load(Ordering::Relaxed);
        assert_eq!(timeout_or_failed, 1, "Should have exactly one compilation timeout or failure");
    }
}
