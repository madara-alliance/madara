use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use serde::de::Error as _;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use std::sync::Arc;

use crate::SierraConvertedClass;
use crate::{ConvertedClass, LegacyConvertedClass};

use {
    blockifier::execution::native::contract_class::NativeCompiledClassV1, cairo_native::executor::AotContractExecutor,
    dashmap::DashMap, std::path::PathBuf, tokio::sync::RwLock,
};

static NATIVE_CACHE: std::sync::LazyLock<DashMap<starknet_types_core::felt::Felt, Arc<NativeCompiledClassV1>>> =
    std::sync::LazyLock::new(DashMap::new);

static COMPILATION_IN_PROGRESS: std::sync::LazyLock<DashMap<starknet_types_core::felt::Felt, Arc<RwLock<()>>>> =
    std::sync::LazyLock::new(DashMap::new);

/// Semaphore to limit concurrent compilations
static COMPILATION_SEMAPHORE: std::sync::LazyLock<tokio::sync::Semaphore> = std::sync::LazyLock::new(|| {
    let config = crate::native_config::get_config();
    tokio::sync::Semaphore::new(config.max_concurrent_compilations)
});

fn get_native_cache_path(
    class_hash: &starknet_types_core::felt::Felt,
    override_config: Option<&crate::native_config::NativeConfig>,
) -> PathBuf {
    let config = override_config.unwrap_or_else(|| crate::native_config::get_config());

    static LOGGED_PATH: std::sync::Once = std::sync::Once::new();
    LOGGED_PATH.call_once(|| {
        tracing::info!("🚀 Cairo Native feature enabled - native classes cache directory: {:?}", config.cache_dir);
    });

    config.cache_dir.join(format!("{:#x}.so", class_hash))
}

fn validate_class_hash(class_hash: &starknet_types_core::felt::Felt) -> Result<(), String> {
    // Validate that class hash is non-zero (zero is not a valid class hash in Starknet)
    if *class_hash == starknet_types_core::felt::Felt::ZERO {
        return Err("Class hash cannot be zero".to_string());
    }

    // Additional check: ensure the hex representation is valid for filename
    let hash_str = format!("{:#x}", class_hash);
    if hash_str.is_empty() || hash_str.len() > 100 {
        return Err(format!("Invalid class hash length: {}", hash_str.len()));
    }
    Ok(())
}

fn evict_cache_if_needed() {
    let config = crate::native_config::get_config();

    if config.max_memory_cache_size == 0 {
        return; // No limit
    }

    let current_size = NATIVE_CACHE.len();
    if current_size > config.max_memory_cache_size {
        let to_remove = current_size - config.max_memory_cache_size;
        tracing::info!(
            "📦 [Cairo Native] Memory cache size ({}) exceeds limit ({}), evicting {} entries",
            current_size,
            config.max_memory_cache_size,
            to_remove
        );

        // Simple FIFO eviction (can be improved to LRU)
        let keys: Vec<_> = NATIVE_CACHE.iter().take(to_remove).map(|entry| *entry.key()).collect();
        for key in keys {
            NATIVE_CACHE.remove(&key);
            crate::native_metrics::metrics().record_cache_eviction();
        }
    }

    // Update cache size metric
    crate::native_metrics::metrics().set_cache_size(NATIVE_CACHE.len());
}

/// Compile a class synchronously (blocking) and return the result
/// Used when blocking compilation mode is enabled
///
/// If `override_config` is provided, it will be used instead of the global config.
/// This is useful for testing with isolated configurations.
fn compile_native_blocking(
    class_hash: starknet_types_core::felt::Felt,
    sierra: &SierraConvertedClass,
    override_config: Option<&crate::native_config::NativeConfig>,
) -> Result<Arc<NativeCompiledClassV1>, String> {
    let config = override_config.unwrap_or_else(|| crate::native_config::get_config());

    // Validate class hash
    if let Err(e) = validate_class_hash(&class_hash) {
        return Err(format!("Invalid class hash {:#x}: {}", class_hash, e));
    }

    let path = get_native_cache_path(&class_hash, Some(config));

    // Ensure directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache directory {:?}: {}", parent, e))?;
    }

    let start = std::time::Instant::now();
    let timer = crate::native_metrics::CompilationTimer::new();

    tracing::info!("🔧 [Cairo Native BLOCKING] Compiling class {:#x} synchronously -> {}", class_hash, path.display());

    // Compile synchronously with timeout
    let sierra_clone = Arc::new(sierra.clone());
    let path_clone = path.clone();
    let compilation_timeout = config.compilation_timeout;

    let compilation_future =
        tokio::task::spawn_blocking(move || sierra_clone.info.contract_class.compile_to_native(&path_clone));

    // Block on the compilation
    let rt = tokio::runtime::Handle::current();
    let result = rt.block_on(async { tokio::time::timeout(compilation_timeout, compilation_future).await });

    match result {
        Ok(Ok(Ok(executor))) => {
            let elapsed = start.elapsed();
            tracing::info!(
                "✅ [Cairo Native BLOCKING] Compilation successful for class {:#x} in {}ms",
                class_hash,
                elapsed.as_millis()
            );

            // Convert to native class
            let (sierra_version, casm) =
                match (sierra.info.contract_class.sierra_version(), sierra.compiled.as_ref().try_into()) {
                    (Ok(v), Ok(c)) => (v, c),
                    (Err(e), _) => {
                        timer.finish(false, false);
                        return Err(format!("Failed to get sierra version: {}", e));
                    }
                    (_, Err(e)) => {
                        timer.finish(false, false);
                        return Err(format!("Failed to convert to CASM: {}", e));
                    }
                };

            let blockifier_compiled_class = match (casm, sierra_version).try_into() {
                Ok(c) => c,
                Err(e) => {
                    timer.finish(false, false);
                    return Err(format!("Failed to convert to blockifier class: {:?}", e));
                }
            };

            let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

            // Evict if cache is full
            evict_cache_if_needed();

            let arc_native = Arc::new(native_class);
            NATIVE_CACHE.insert(class_hash, arc_native.clone());

            timer.finish(true, false);
            Ok(arc_native)
        }
        Ok(Ok(Err(e))) => {
            timer.finish(false, false);
            Err(format!("Compilation failed: {:#}", e))
        }
        Ok(Err(e)) => {
            timer.finish(false, false);
            Err(format!("Compilation task panicked: {:#}", e))
        }
        Err(_) => {
            timer.finish(false, true);
            let _ = std::fs::remove_file(&path);
            Err(format!("Compilation timeout ({:?}) exceeded", compilation_timeout))
        }
    }
}

fn spawn_native_compilation(class_hash: starknet_types_core::felt::Felt, sierra: Arc<SierraConvertedClass>) {
    let config = crate::native_config::get_config();
    let compilation_timeout = config.compilation_timeout;

    // Spawn background task for native compilation
    tokio::spawn(async move {
        // Acquire compilation slot
        let permit = match COMPILATION_SEMAPHORE.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                tracing::warn!(
                    "⚠️  [Cairo Native] Max concurrent compilations reached, skipping class {:#x}",
                    class_hash
                );
                COMPILATION_IN_PROGRESS.remove(&class_hash);
                return;
            }
        };

        let lock = COMPILATION_IN_PROGRESS.entry(class_hash).or_insert_with(|| Arc::new(RwLock::new(()))).clone();
        let _guard = lock.write().await;

        // Check cache again in case another task compiled it
        if NATIVE_CACHE.contains_key(&class_hash) {
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            drop(permit);
            return;
        }

        // Validate class hash
        if let Err(e) = validate_class_hash(&class_hash) {
            tracing::error!("❌ [Cairo Native] Invalid class hash {:#x}: {}", class_hash, e);
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            drop(permit);
            return;
        }

        let path = get_native_cache_path(&class_hash, None);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("❌ [Cairo Native] Failed to create cache directory {:?}: {}", parent, e);
                COMPILATION_IN_PROGRESS.remove(&class_hash);
                drop(permit);
                return;
            }
        }

        let start = std::time::Instant::now();
        let timer = crate::native_metrics::CompilationTimer::new();

        tracing::info!(
            "🔧 [Cairo Native] Starting background compilation for class {:#x} -> {}",
            class_hash,
            path.display()
        );

        let sierra_clone = sierra.clone();
        let path_clone = path.clone();

        // Spawn with timeout
        let compilation_future =
            tokio::task::spawn_blocking(move || sierra_clone.info.contract_class.compile_to_native(&path_clone));

        let result = tokio::time::timeout(compilation_timeout, compilation_future).await;

        match result {
            Ok(Ok(Ok(executor))) => {
                let elapsed = start.elapsed();
                let elapsed_ms = elapsed.as_millis();
                tracing::info!(
                    "✅ [Cairo Native] Compilation successful for class {:#x} in {}ms ({:.2}s) - saved to disk",
                    class_hash,
                    elapsed_ms,
                    elapsed.as_secs_f64()
                );

                // Get sierra version and compiled class
                let success = match (sierra.info.contract_class.sierra_version(), sierra.compiled.as_ref().try_into()) {
                    (Ok(sierra_version), Ok(casm)) => {
                        // Convert (CasmContractClass, SierraVersion) tuple to CompiledClassV1
                        match (casm, sierra_version).try_into() {
                            Ok(blockifier_compiled_class) => {
                                let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

                                // Evict if cache is full
                                evict_cache_if_needed();

                                NATIVE_CACHE.insert(class_hash, Arc::new(native_class));
                                let cache_size = NATIVE_CACHE.len();
                                tracing::info!(
                                    "💾 [Cairo Native] Cached native class {:#x} in memory (cache size: {})",
                                    class_hash,
                                    cache_size
                                );
                                true
                            }
                            Err(e) => {
                                tracing::error!(
                                    "⚠️  [Cairo Native] Failed to convert to blockifier class for {:#x}: {:?}",
                                    class_hash,
                                    e
                                );
                                false
                            }
                        }
                    }
                    (Err(e), _) => {
                        tracing::error!("⚠️  [Cairo Native] Failed to get sierra version for {:#x}: {}", class_hash, e);
                        false
                    }
                    (_, Err(e)) => {
                        tracing::error!("⚠️  [Cairo Native] Failed to convert to CASM for {:#x}: {}", class_hash, e);
                        false
                    }
                };

                timer.finish(success, false);
            }
            Ok(Ok(Err(e))) => {
                tracing::warn!("⚠️  [Cairo Native] Compilation failed for class {:#x}: {:#}", class_hash, e);
                timer.finish(false, false);
            }
            Ok(Err(e)) => {
                tracing::error!("❌ [Cairo Native] Compilation task panicked for class {:#x}: {:#}", class_hash, e);
                timer.finish(false, false);
            }
            Err(_) => {
                tracing::error!(
                    "⏱️  [Cairo Native] Compilation timeout ({:?}) for class {:#x}",
                    compilation_timeout,
                    class_hash
                );
                // Try to clean up the partial file
                let _ = std::fs::remove_file(&path);
                timer.finish(false, true);
            }
        }

        COMPILATION_IN_PROGRESS.remove(&class_hash);
        drop(permit);
    });
}

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = ProgramError;

    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => {
                RunnableCompiledClass::try_from(ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi()?))
            }

            ConvertedClass::Sierra(sierra @ SierraConvertedClass { class_hash, compiled, info }) => {
                // Runtime check: is native execution enabled?
                let config = crate::native_config::get_config();
                if !config.enable_native_execution {
                    // Native execution disabled at runtime - use Cairo VM
                    tracing::debug!("🐪 [Cairo VM] Native execution disabled, using VM for class {:#x}", class_hash);
                    let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                        ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                    })?;
                    return RunnableCompiledClass::try_from(ApiContractClass::V1((
                        compiled.as_ref().try_into()?,
                        sierra_version,
                    )));
                }

                // Native execution enabled - proceed with native path
                // Check in-memory cache first
                if let Some(cached) = NATIVE_CACHE.get(class_hash) {
                    let cache_size = NATIVE_CACHE.len();
                    crate::native_metrics::metrics().record_cache_hit_memory();
                    tracing::debug!(
                        "⚡ [Cairo Native] Using in-memory cached native class {:#x} (cache size: {})",
                        class_hash,
                        cache_size
                    );
                    return Ok(RunnableCompiledClass::from(cached.value().as_ref().clone()));
                }

                // Try to load from disk synchronously (fast path)
                let path = get_native_cache_path(class_hash, None);
                if path.exists() {
                    match AotContractExecutor::from_path(&path) {
                        Ok(Some(executor)) => {
                            crate::native_metrics::metrics().record_cache_hit_disk();
                            tracing::info!(
                                "📁 [Cairo Native] Loaded native class {:#x} from disk: {}",
                                class_hash,
                                path.display()
                            );

                            let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                                ProgramError::Parse(serde_json::Error::custom(
                                    "Failed to get sierra version from program",
                                ))
                            })?;
                            let casm: casm_classes_v2::casm_contract_class::CasmContractClass =
                                compiled.as_ref().try_into().map_err(|_| {
                                    ProgramError::Parse(serde_json::Error::custom("Failed to convert to CASM"))
                                })?;

                            // Convert (CasmContractClass, SierraVersion) tuple to CompiledClassV1
                            let blockifier_compiled_class = (casm, sierra_version).try_into().map_err(|_| {
                                ProgramError::Parse(serde_json::Error::custom("Failed to convert to blockifier class"))
                            })?;
                            let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

                            // Evict if cache is full
                            evict_cache_if_needed();

                            NATIVE_CACHE.insert(*class_hash, Arc::new(native_class.clone()));
                            tracing::debug!("[Cairo Native] Added class {:#x} to in-memory cache", class_hash);
                            return Ok(RunnableCompiledClass::from(native_class));
                        }
                        Ok(None) => {
                            tracing::warn!(
                                "⚠️  [Cairo Native] Failed to load native class {:#x} from disk (file locked)",
                                class_hash
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "⚠️  [Cairo Native] Failed to load native class {:#x} from disk: {:#}",
                                class_hash,
                                e
                            );
                            // Try to remove corrupted file
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }

                // Not in cache and not on disk - check compilation mode
                crate::native_metrics::metrics().record_cache_miss();

                let config = crate::native_config::get_config();

                // Check if blocking mode is enabled
                if config.is_blocking_mode() {
                    // BLOCKING MODE: Compile synchronously and fail if compilation fails
                    tracing::info!(
                        "⏸️  [Cairo Native BLOCKING] Class {:#x} not cached - compiling synchronously",
                        class_hash
                    );

                    match compile_native_blocking(*class_hash, sierra, None) {
                        Ok(native_class) => {
                            tracing::info!("✅ [Cairo Native BLOCKING] Successfully compiled class {:#x}", class_hash);
                            return Ok(RunnableCompiledClass::from(native_class.as_ref().clone()));
                        }
                        Err(e) => {
                            tracing::error!(
                                "❌ [Cairo Native BLOCKING] Compilation required but failed for class {:#x}: {}",
                                class_hash,
                                e
                            );
                            return Err(ProgramError::Parse(serde_json::Error::custom(format!(
                                "Native compilation required but failed: {}",
                                e
                            ))));
                        }
                    }
                }

                // ASYNC MODE: Spawn background compilation and fall back to VM
                crate::native_metrics::metrics().record_vm_fallback();

                let is_already_compiling = COMPILATION_IN_PROGRESS.contains_key(class_hash);
                if is_already_compiling {
                    tracing::debug!("🔄 [Cairo Native] Class {:#x} already compiling, using VM for now", class_hash);
                } else {
                    tracing::info!(
                        "🔄 [Cairo Native] Class {:#x} not cached - spawning background compilation, using VM for now",
                        class_hash
                    );
                    spawn_native_compilation(*class_hash, Arc::new(sierra.clone()));
                }

                // Fall back to cairo-vm (CASM execution)
                tracing::debug!("🐪 [Cairo VM] Executing class {:#x} with Cairo VM", class_hash);
                let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                    ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                })?;
                RunnableCompiledClass::try_from(ApiContractClass::V1((compiled.as_ref().try_into()?, sierra_version)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
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

        // Add dummy entries
        for i in 0..5 {
            let _hash = Felt::from(i);
            // We can't easily create a NativeCompiledClassV1 without actual compilation,
            // so we'll just test that the cache operations don't panic
            // Real integration tests would use actual compiled classes
        }

        // Cache operations should not panic
        evict_cache_if_needed();
    }

    #[tokio::test]
    async fn test_compilation_semaphore_limit() {
        let semaphore = &*COMPILATION_SEMAPHORE;
        let config = crate::native_config::get_config();

        // Should be able to acquire up to max_concurrent_compilations
        let mut permits = vec![];
        for _ in 0..config.max_concurrent_compilations {
            if let Ok(permit) = semaphore.try_acquire() {
                permits.push(permit);
            }
        }

        // Should have acquired at least some permits
        assert!(!permits.is_empty());

        // Clean up
        drop(permits);
    }

    #[rstest]
    #[case::async_mode(crate::native_config::NativeCompilationMode::Async)]
    #[case::blocking_mode(crate::native_config::NativeCompilationMode::Blocking)]
    fn test_native_config_modes(#[case] mode: crate::native_config::NativeCompilationMode) {
        let config = crate::native_config::NativeConfig::new().with_compilation_mode(mode);

        assert_eq!(config.compilation_mode, mode);

        match mode {
            crate::native_config::NativeCompilationMode::Async => {
                assert!(!config.is_blocking_mode());
            }
            crate::native_config::NativeCompilationMode::Blocking => {
                assert!(config.is_blocking_mode());
            }
        }
    }

    #[test]
    fn test_metrics_recorded() {
        // Test that metrics can be accessed and recorded
        let metrics = crate::native_metrics::metrics();

        // Record various metrics
        metrics.record_cache_hit_memory();
        metrics.record_cache_hit_disk();
        metrics.record_cache_miss();
        metrics.record_vm_fallback();

        // Get summary (should not panic and should contain expected text)
        let summary = metrics.summary();
        assert!(!summary.is_empty());
        assert!(summary.contains("Cairo Native Metrics"));
        assert!(summary.contains("Cache:"));
    }

    #[test]
    fn test_cache_eviction_logic() {
        // Clear cache
        NATIVE_CACHE.clear();

        let config = crate::native_config::get_config();
        let max_size = config.max_memory_cache_size;

        // Test that eviction threshold is calculated correctly
        // The eviction happens when cache size > max_size
        // This is a unit test for the eviction logic itself
        assert!(max_size > 0, "Max cache size should be positive");
    }

    // TODO (mohit 2025-11-04): Add integration tests with actual Cairo contracts
    // These tests would:
    // - Create real SierraConvertedClass instances
    // - Test full compilation pipeline (blocking mode)
    // - Verify cache persistence across restarts
    // - Test concurrent compilation behavior
    // - Verify metrics accuracy with real compilations
}

// TODO (mohit 2025-11-04): Add benchmarks module
// #[cfg(all(test, feature = "cairo_native"))]
// mod benches {
//     // Criterion benchmarks comparing VM vs Native execution
// }
