//! Execution flow for Cairo Native classes
//!
//! This module handles the main execution flow for Sierra classes with native execution support.

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use mp_convert::ToFelt;
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
        native_enabled = config.enable_native_execution,
        "handle_sierra_class"
    );

    // If native execution is disabled, use VM immediately
    if !config.enable_native_execution {
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
    // Skip disk cache if compilation is in progress to avoid loading incomplete files
    if compilation::COMPILATION_IN_PROGRESS.contains_key(class_hash) {
        let elapsed = start.elapsed();
        super::metrics::metrics().record_compilation_in_progress_skip();
        tracing::debug!(
            target: "madara_cairo_native",
            class_hash = %format!("{:#x}", class_hash.to_felt()),
            elapsed = ?elapsed,
            elapsed_ms = elapsed.as_millis(),
            "compilation_in_progress_skipping_disk_cache"
        );
        return None;
    }

    match cache::try_get_from_disk_cache(class_hash, sierra, config) {
        Ok(Some(cached)) => Some(cached),
        Ok(None) => {
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

/// Handles compilation in blocking mode.
///
/// Compiles synchronously and returns the native class, or fails if compilation fails.
fn handle_blocking_compilation(
    class_hash: &ClassHash,
    sierra: &SierraConvertedClass,
    config: &Arc<config::NativeConfig>,
    start: Instant,
) -> Result<RunnableCompiledClass, ProgramError> {
    super::metrics::metrics().record_compilation_blocking_miss();
    tracing::debug!(
        target: "madara_cairo_native",
        class_hash = %format!("{:#x}", class_hash.to_felt()),
        "compilation_blocking_miss"
    );

    match compilation::compile_native_blocking(*class_hash, sierra, config.as_ref()) {
        Ok(native_class) => {
            let conversion_start = Instant::now();
            let runnable = RunnableCompiledClass::from(native_class.as_ref().clone());
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
            // In blocking mode, failures are fatal
            tracing::error!(
                target: "madara_cairo_native",
                class_hash = %format!("{:#x}", class_hash.to_felt()),
                error = %e,
                "compilation_blocking_failed"
            );
            Err(e.into())
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
    let is_failed = compilation::FAILED_COMPILATIONS.contains_key(class_hash);

    // Handle retry logic based on config
    if is_failed {
        if config.enable_retry {
            // Retry enabled: remove from failed list and retry compilation
            compilation::FAILED_COMPILATIONS.remove(class_hash);
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
    use crate::metrics::test_counters;
    use mp_class::{FlattenedSierraClass, SierraClassInfo, SierraConvertedClass};
    use starknet_api::core::ClassHash;
    use starknet_types_core::felt::Felt;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    // Test mutex to serialize test execution for tests that need to clear/modify caches
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    // Counter for generating unique test class hashes
    static TEST_CLASS_HASH_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    // Helper function to create a unique test class hash
    fn create_unique_test_class_hash() -> ClassHash {
        let counter = TEST_CLASS_HASH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = Felt::from_hex_unchecked("0x2000000000000000000000000000000000000000000000000000000000000000");
        ClassHash(base + Felt::from(counter))
    }

    // Helper function to create a test SierraConvertedClass (reuse from cache.rs tests)
    fn create_test_sierra_class() -> Result<SierraConvertedClass, Box<dyn std::error::Error>> {
        use m_cairo_test_contracts::TEST_CONTRACT_SIERRA;
        use mp_class::CompiledSierra;
        use serde_json::Value;

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

    // Helper function to create a test NativeCompiledClassV1 and insert into memory cache
    fn create_and_cache_native_class(
        sierra: &SierraConvertedClass,
        temp_dir: &TempDir,
        class_hash: ClassHash,
        _config: &config::NativeConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use blockifier::execution::native::contract_class::NativeCompiledClassV1;
        use std::time::Instant;

        // Create path for compiled .so file
        let so_path = temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()));

        // Compile Sierra to native
        let executor = sierra.info.contract_class.compile_to_native(&so_path)?;

        // Convert Sierra to blockifier compiled class
        let blockifier_compiled_class = crate::compilation::convert_sierra_to_blockifier_class(sierra)?;

        // Create NativeCompiledClassV1
        let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

        // Insert into memory cache
        cache::NATIVE_CACHE.insert(class_hash, (Arc::new(native_class), Instant::now()));

        Ok(())
    }

    // Helper function to create and save a native class to disk (simulates previous compilation)
    fn create_and_save_native_class_to_disk(
        sierra: &SierraConvertedClass,
        temp_dir: &TempDir,
        class_hash: ClassHash,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use blockifier::execution::native::contract_class::NativeCompiledClassV1;

        // Create path for compiled .so file
        let so_path = temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()));

        // Compile Sierra to native and save to disk
        let executor = sierra.info.contract_class.compile_to_native(&so_path)?;

        // Convert Sierra to blockifier compiled class
        let blockifier_compiled_class = crate::compilation::convert_sierra_to_blockifier_class(sierra)?;

        // Create NativeCompiledClassV1 (this validates the executor can be loaded)
        let _native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

        // File is already saved to disk by compile_to_native
        // Verify file exists
        assert!(so_path.exists(), "Native class file should exist on disk");

        Ok(())
    }

    // Helper function to create a test config with native enabled
    fn create_test_config(temp_dir: &TempDir, mode: config::NativeCompilationMode) -> Arc<config::NativeConfig> {
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(mode)
                .with_compilation_timeout(Duration::from_secs(30)),
        );

        // Initialize compilation semaphore for async tests
        compilation::init_compilation_semaphore(config.max_concurrent_compilations);

        config
    }

    // Helper function to poll for compilation completion with timeout (async version)
    // Returns true if compilation completed and class is cached, false if timeout
    async fn wait_for_compilation_completion_async(class_hash: &ClassHash, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);
        let check_interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            // Check if compilation completed and class is in cache
            if cache::NATIVE_CACHE.contains_key(class_hash) {
                // Give it a small delay to ensure it's fully cached
                tokio::time::sleep(Duration::from_millis(50)).await;
                return true;
            }

            // Check if compilation is still in progress
            if !compilation::COMPILATION_IN_PROGRESS.contains_key(class_hash) {
                // Compilation finished but not cached - might have failed
                // Wait a bit more to see if it gets cached
                tokio::time::sleep(check_interval).await;
                if cache::NATIVE_CACHE.contains_key(class_hash) {
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

    #[test]
    fn test_handle_sierra_class_memory_cache_hit() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Pre-populate memory cache
        create_and_cache_native_class(&sierra, &temp_dir, class_hash, &config)
            .expect("Failed to create and cache native class");

        // Verify it's in cache before calling handle_sierra_class
        assert!(cache::NATIVE_CACHE.contains_key(&class_hash), "Class should be in memory cache before call");

        // Memory cache hit expected
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should successfully retrieve from memory cache");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Verify it's still in cache after call
        assert!(cache::NATIVE_CACHE.contains_key(&class_hash), "Class should remain in memory cache after call");

        // Metrics assertions - memory cache hit, no disk access, no compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache hit"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            0,
            "Should have no memory cache misses (memory hit)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (memory hit, disk not checked)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (disk not checked)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have no compilations (memory cache hit)"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            0,
            "Should have no VM fallbacks (native execution)"
        );
    }

    #[test]
    fn test_handle_sierra_class_memory_miss_disk_hit() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Create and save native class to disk (simulate previous compilation)
        create_and_save_native_class_to_disk(&sierra, &temp_dir, class_hash)
            .expect("Failed to create and save native class to disk");

        // Verify memory cache still doesn't have it
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache before disk load");

        // Disk cache hit expected
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should successfully retrieve from disk cache");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Verify it was loaded into memory cache after disk hit
        assert!(cache::NATIVE_CACHE.contains_key(&class_hash), "Should be loaded into memory cache after disk hit");

        // Metrics assertions - disk cache hit, no compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (memory missed)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed), 1, "Should have exactly one disk cache hit");
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (disk hit)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have no compilations (disk cache hit)"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            0,
            "Should have no VM fallbacks (native execution from disk)"
        );
    }

    #[test]
    fn test_handle_sierra_class_native_disabled() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create config with native disabled
        let config = Arc::new(
            config::NativeConfig::default().with_native_execution(false).with_cache_dir(temp_dir.path().to_path_buf()),
        );

        let _metrics_guard = test_counters::acquire_and_reset();

        // VM class returned immediately when native is disabled
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should return VM class when native is disabled");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Metrics assertions - native disabled
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no cache hits (native disabled)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            0,
            "Should have no memory cache misses (native disabled, no cache lookup)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (native disabled)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (native disabled, no cache lookup)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have no compilations (native disabled)"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            0,
            "Should have no VM fallbacks (native disabled)"
        );
    }

    #[test]
    fn test_handle_sierra_class_cache_miss_blocking_mode() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();

        // Ensure our specific class_hash is not in any caches (cleanup from any previous runs)
        cache::NATIVE_CACHE.remove(&class_hash);
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        compilation::FAILED_COMPILATIONS.remove(&class_hash);

        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Blocking);

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );

        // Reset metrics RIGHT BEFORE the operation to minimize window for other tests to interfere
        // This ensures metrics from cache tests running in parallel don't leak in
        let _metrics_guard = test_counters::acquire_and_reset();

        // Blocking mode compiles synchronously
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should successfully compile in blocking mode");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Verify it was cached
        assert!(cache::NATIVE_CACHE.contains_key(&class_hash), "Should be cached after compilation");
        // Compilation should be complete, so not in progress
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Metrics assertions - cache miss and blocking compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache miss"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation in blocking mode"
        );
        assert_eq!(
            test_counters::COMPILATION_BLOCKING_COMPLETE.load(Ordering::Relaxed),
            1,
            "Should have exactly one blocking compilation completion"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            0,
            "Should have no VM fallbacks (blocking mode waits for compilation)"
        );
    }

    #[tokio::test]
    async fn test_handle_sierra_class_cache_miss_async_mode() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should not be in failed_compilations initially"
        );

        let _metrics_guard = test_counters::acquire_and_reset();

        // Async mode returns VM immediately while compilation happens in background
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should return VM class immediately in async mode");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Wait for compilation completion with 20 second timeout
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation completed and class is cached
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should not be in failed_compilations after successful compilation"
        );

        // Metrics assertions - cache miss and async compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache miss"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            1,
            "Should have exactly one VM fallback (async mode returns VM immediately)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation after async completion"
        );
    }

    #[tokio::test]
    async fn test_handle_sierra_class_disk_cache_error() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // Create a corrupt/invalid .so file on disk to trigger disk cache error
        let so_path = temp_dir.path().join(format!("{:#x}.so", class_hash.to_felt()));
        std::fs::write(&so_path, b"invalid binary data").expect("Failed to write invalid file");

        // Disk error handled gracefully, falls back to compilation
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // In async mode, should return VM class even if disk cache fails
        assert!(result.is_ok(), "Should handle disk cache error gracefully");

        // Wait for compilation completion with 20 second timeout
        // Compilation spawned in background after disk error
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds after disk cache error");

        // Compilation completed successfully despite disk error - verify it's in cache
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should not be in failed_compilations after successful compilation"
        );

        // Metrics assertions - disk error and fallback to compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (disk load error)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (disk errored, not missed)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_LOAD_ERROR.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk load error"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            1,
            "Should have exactly one VM fallback (async mode returns VM immediately)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation after disk error"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation after disk error"
        );
    }

    #[test]
    fn test_handle_sierra_class_compilation_in_progress_skip_disk() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // Mark compilation as in progress
        use tokio::sync::RwLock;
        compilation::COMPILATION_IN_PROGRESS.insert(class_hash, Arc::new(RwLock::new(())));
        assert!(
            compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should be in compilation_in_progress"
        );

        // Disk cache skipped when compilation is in progress
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should return VM class when compilation is in progress");

        // Metrics assertions - compilation in progress, disk skipped

        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::COMPILATION_IN_PROGRESS_SKIP.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache skip due to compilation in progress"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            1,
            "Should have exactly one VM fallback (async mode returns VM when compilation in progress)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (memory cache missed)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (disk lookup was skipped)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (disk lookup was skipped)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have no new compilations started (already in progress)"
        );

        // Clean up
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should be removed from compilation_in_progress"
        );
    }

    #[test]
    fn test_handle_sierra_class_blocking_compilation_failure() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Create config with very short timeout to force compilation failure
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Blocking)
                .with_compilation_timeout(Duration::from_millis(1)), // Very short timeout
        );

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should not be in failed_compilations initially"
        );

        // Compilation timeout expected in blocking mode with very short timeout
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_err(), "Should return an error when compilation fails");

        // Expected: compilation timeout/failure in blocking mode
        // Verify class is not cached after failure
        assert!(
            !cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should not be in memory cache after compilation failure"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after failure"
        );
        // In blocking mode, failures don't go to FAILED_COMPILATIONS (that's async mode only)

        // Metrics assertions - compilation timeout/failure
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache miss"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        let timeout_or_failed = test_counters::COMPILATIONS_TIMEOUT.load(Ordering::Relaxed)
            + test_counters::COMPILATIONS_FAILED.load(Ordering::Relaxed);
        assert_eq!(timeout_or_failed, 1, "Should have exactly one compilation timeout or failure");
    }

    #[tokio::test]
    async fn test_handle_sierra_class_multiple_calls_same_class() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // First call triggers compilation and returns VM
        let result1 =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());
        assert!(result1.is_ok(), "First call should succeed");

        // Poll for compilation completion with 20 second timeout
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        // Metrics assertions - first call missed, second call hit memory
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss (first call)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache miss (first call)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (first call)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (first call)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation (first call)"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            1,
            "Should have exactly one VM fallback (first call)"
        );

        // Test should fail if compilation doesn't complete within timeout
        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Compilation completed - verify it's in cache
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Reset metrics before second call to isolate second call metrics
        test_counters::reset_all();

        // Second call hits cache since compilation completed
        let result2 =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());
        assert!(result2.is_ok(), "Second call should succeed");

        // Small delay to ensure cache was checked
        tokio::time::sleep(Duration::from_millis(1000)).await;

        // Metrics assertions - second call hit memory cache
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            0,
            "Should have no memory cache misses (second call)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (second call)"
        );
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache hit"
        );
        assert_eq!(test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed), 0, "Should have no disk cache hits");
        assert_eq!(test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed), 0, "Should have no compilations");
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            0,
            "Should have no successful compilations"
        );
        assert_eq!(test_counters::VM_FALLBACKS.load(Ordering::Relaxed), 0, "Should have no VM fallbacks");
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
    #[test]
    fn test_handle_sierra_class_memory_cache_timeout() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Configure an extremely short memory cache timeout to reliably trigger timeout
        // 1 nanosecond timeout ensures timeout occurs since conversion operations take longer
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_memory_cache_timeout(Duration::from_nanos(1)), // Extremely short timeout to force timeout
        );

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // Pre-populate disk cache to enable testing disk fallback after memory timeout
        create_and_save_native_class_to_disk(&sierra, &temp_dir, class_hash)
            .expect("Failed to create and save native class to disk");

        // Memory cache lookup will timeout, triggering fallback to disk cache
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        assert!(result.is_ok(), "Should successfully retrieve from disk cache after memory miss");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Verify it was loaded into memory cache after disk hit
        assert!(cache::NATIVE_CACHE.contains_key(&class_hash), "Should be loaded into memory cache after disk hit");

        // Metrics assertions - memory timeout, disk hit, no compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (memory timed out)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_TIMEOUT.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache timeout"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache hit (fallback after memory timeout)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            0,
            "Should have no disk cache misses (disk hit)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have no compilations (disk cache hit)"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            0,
            "Should have no VM fallbacks (native execution from disk)"
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
    #[tokio::test]
    async fn test_handle_sierra_class_disk_cache_timeout() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Configure an extremely short disk cache timeout to reliably trigger timeout
        // 1 microsecond timeout ensures timeout occurs since disk I/O operations take longer
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_disk_cache_load_timeout(Duration::from_micros(1)), // Extremely short timeout to force timeout
        );

        // Verify memory cache doesn't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // Pre-populate disk cache to enable testing timeout during disk load operation
        // With 1 microsecond timeout, disk load will timeout since loading .so files takes longer
        create_and_save_native_class_to_disk(&sierra, &temp_dir, class_hash)
            .expect("Failed to create and save native class to disk");

        // Memory cache misses, disk cache load times out (file exists but load operation times out),
        // triggering fallback to compilation
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // Async mode returns VM immediately while compilation happens in background
        assert!(result.is_ok(), "Should return VM class immediately in async mode");
        let runnable = result.unwrap();
        assert!(std::mem::size_of_val(&runnable) > 0, "Should return valid RunnableCompiledClass");

        // Wait for compilation completion with 20 second timeout
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;

        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation completed and class is cached
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Metrics assertions - disk timeout and fallback to compilation
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::CACHE_HITS_MEMORY.load(Ordering::Relaxed),
            0,
            "Should have no memory cache hits (cache miss)"
        );
        assert_eq!(
            test_counters::CACHE_MEMORY_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one memory cache miss"
        );
        assert_eq!(
            test_counters::CACHE_HITS_DISK.load(Ordering::Relaxed),
            0,
            "Should have no disk cache hits (disk timed out)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_MISS.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache miss (timeout treated as miss)"
        );
        assert_eq!(
            test_counters::CACHE_DISK_LOAD_TIMEOUT.load(Ordering::Relaxed),
            1,
            "Should have exactly one disk cache load timeout"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            1,
            "Should have exactly one VM fallback (async mode returns VM immediately)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation after disk timeout"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation"
        );
    }

    #[tokio::test]
    async fn test_compilation_in_progress_prevents_duplicate() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );

        // Spawn multiple concurrent requests for the same class
        let num_concurrent_requests = 10;
        let mut handles = vec![];

        for _ in 0..num_concurrent_requests {
            let sierra_clone = sierra.clone();
            let class_hash_clone = class_hash;
            let config_clone = config.clone();
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
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify only one compilation was started
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation, not {}",
            num_concurrent_requests
        );

        // Verify only one entry in COMPILATION_IN_PROGRESS
        assert_eq!(compilation::COMPILATION_IN_PROGRESS.len(), 1, "Should have exactly one compilation in progress");
        assert!(
            compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should be in compilation_in_progress"
        );

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
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );

        // Verify metrics - only one compilation started and succeeded
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation"
        );
        assert_eq!(
            test_counters::VM_FALLBACKS.load(Ordering::Relaxed),
            num_concurrent_requests as u64,
            "Should have {} VM fallbacks (one per concurrent request)",
            num_concurrent_requests
        );
    }

    #[tokio::test]
    async fn test_compilation_in_progress_cleanup() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = create_test_config(&temp_dir, config::NativeCompilationMode::Async);

        // Clear any existing state
        cache::NATIVE_CACHE.remove(&class_hash);
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        compilation::FAILED_COMPILATIONS.remove(&class_hash);

        // Verify initial state - not in progress
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress initially"
        );
        assert_eq!(compilation::COMPILATION_IN_PROGRESS.len(), 0, "Should have no compilations in progress initially");

        // Trigger async compilation
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // Should return VM version immediately (async mode)
        assert!(result.is_ok(), "Should return VM class immediately in async mode");

        // Wait a short time to allow compilation to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify compilation started and entry is in COMPILATION_IN_PROGRESS
        assert!(
            compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should be in compilation_in_progress after spawning"
        );
        assert_eq!(compilation::COMPILATION_IN_PROGRESS.len(), 1, "Should have exactly one compilation in progress");

        // Wait for compilation to complete (with timeout)
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;
        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation completed successfully
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after compilation completes"
        );

        // Verify cleanup - entry should be removed from COMPILATION_IN_PROGRESS
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Class should not be in compilation_in_progress after completion"
        );
        assert_eq!(
            compilation::COMPILATION_IN_PROGRESS.len(),
            0,
            "Should have no compilations in progress after completion"
        );

        // Verify metrics - compilation should have succeeded
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation"
        );
        assert_eq!(test_counters::COMPILATIONS_FAILED.load(Ordering::Relaxed), 0, "Should have no failed compilations");
    }

    #[tokio::test]
    async fn test_async_mode_retry_enabled() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_compilation_timeout(Duration::from_secs(30))
                .with_enable_retry(true),
        );
        compilation::init_compilation_semaphore(config.max_concurrent_compilations);

        // Clear any existing state
        cache::NATIVE_CACHE.remove(&class_hash);
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        compilation::FAILED_COMPILATIONS.remove(&class_hash);

        // Simulate a previously failed compilation by adding to FAILED_COMPILATIONS
        compilation::FAILED_COMPILATIONS.insert(class_hash, Instant::now());
        assert!(
            compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should be in FAILED_COMPILATIONS initially"
        );

        // Request the class with retry enabled
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // Should return VM version immediately (async mode)
        assert!(result.is_ok(), "Should return VM class immediately in async mode");

        // Wait a short time to allow compilation to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify retry happened: class should be removed from FAILED_COMPILATIONS
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should be removed from FAILED_COMPILATIONS when retry is enabled"
        );

        // Verify compilation was spawned (should be in COMPILATION_IN_PROGRESS)
        assert!(
            compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Compilation should be spawned when retry is enabled"
        );

        // Wait for compilation to complete
        let compilation_completed = wait_for_compilation_completion_async(&class_hash, 20).await;
        assert!(compilation_completed, "Compilation should complete within 20 seconds");

        // Verify compilation succeeded
        assert!(
            cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should be in memory cache after successful retry compilation"
        );
        assert!(
            !compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should not be in FAILED_COMPILATIONS after successful compilation"
        );

        // Verify metrics
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            1,
            "Should have started exactly one compilation (retry)"
        );
        assert_eq!(
            test_counters::COMPILATIONS_SUCCEEDED.load(Ordering::Relaxed),
            1,
            "Should have exactly one successful compilation"
        );
    }

    #[tokio::test]
    async fn test_async_mode_retry_disabled() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Async)
                .with_compilation_timeout(Duration::from_secs(30))
                .with_enable_retry(false),
        );
        compilation::init_compilation_semaphore(config.max_concurrent_compilations);

        // Clear any existing state
        cache::NATIVE_CACHE.remove(&class_hash);
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        compilation::FAILED_COMPILATIONS.remove(&class_hash);

        // Simulate a previously failed compilation by adding to FAILED_COMPILATIONS
        compilation::FAILED_COMPILATIONS.insert(class_hash, Instant::now());
        assert!(
            compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should be in FAILED_COMPILATIONS initially"
        );

        // Request the class with retry disabled
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // Should return VM version immediately (async mode)
        assert!(result.is_ok(), "Should return VM class immediately in async mode");

        // Wait a short time to ensure no compilation was spawned
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify retry did NOT happen: class should still be in FAILED_COMPILATIONS
        assert!(
            compilation::FAILED_COMPILATIONS.contains_key(&class_hash),
            "Class should remain in FAILED_COMPILATIONS when retry is disabled"
        );

        // Verify compilation was NOT spawned (should NOT be in COMPILATION_IN_PROGRESS)
        assert!(
            !compilation::COMPILATION_IN_PROGRESS.contains_key(&class_hash),
            "Compilation should NOT be spawned when retry is disabled"
        );

        // Verify class is NOT in cache (no compilation happened)
        assert!(
            !cache::NATIVE_CACHE.contains_key(&class_hash),
            "Class should NOT be in memory cache when retry is disabled"
        );

        // Verify metrics - no compilation should have started
        use std::sync::atomic::Ordering;
        assert_eq!(
            test_counters::COMPILATIONS_STARTED.load(Ordering::Relaxed),
            0,
            "Should have started no compilations when retry is disabled"
        );
        assert_eq!(test_counters::VM_FALLBACKS.load(Ordering::Relaxed), 1, "Should have exactly one VM fallback");
    }

    #[test]
    fn test_compilation_timeout_config() {
        let _guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _metrics_guard = test_counters::acquire_and_reset();

        let class_hash = create_unique_test_class_hash();
        let sierra = create_test_sierra_class().expect("Failed to create test Sierra class");
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Clear any existing state
        cache::NATIVE_CACHE.remove(&class_hash);
        compilation::COMPILATION_IN_PROGRESS.remove(&class_hash);
        compilation::FAILED_COMPILATIONS.remove(&class_hash);

        // Verify caches don't have our unique class_hash
        assert!(!cache::NATIVE_CACHE.contains_key(&class_hash), "Class should not be in memory cache initially");

        // Create config with a very short timeout to verify timeout behavior
        // In a real scenario, compilations usually complete quickly, so we use 1ms to force timeout
        let configured_timeout = Duration::from_millis(1);
        let config = Arc::new(
            config::NativeConfig::default()
                .with_native_execution(true)
                .with_cache_dir(temp_dir.path().to_path_buf())
                .with_compilation_mode(config::NativeCompilationMode::Blocking)
                .with_compilation_timeout(configured_timeout),
        );

        // Verify the config has the correct timeout value
        assert_eq!(
            config.compilation_timeout, configured_timeout,
            "Config should have the configured timeout value of {:?}",
            configured_timeout
        );

        // Attempt compilation with very short timeout - should timeout
        let result =
            handle_sierra_class(&sierra, &class_hash.to_felt(), &sierra.compiled, &sierra.info, config.clone());

        // Should return an error due to timeout
        assert!(result.is_err(), "Should return an error when compilation times out");

        // Verify timeout metrics were recorded
        use std::sync::atomic::Ordering;
        let timeout_count = test_counters::COMPILATIONS_TIMEOUT.load(Ordering::Relaxed);
        assert!(timeout_count > 0, "Should have recorded at least one compilation timeout");

        // Verify the timeout error matches the configured timeout
        if let Err(e) = result {
            let error_str = format!("{:?}", e);
            assert!(
                error_str.contains("timeout") || error_str.contains("Timeout"),
                "Error should mention timeout, got: {}",
                error_str
            );
        }
    }
}
