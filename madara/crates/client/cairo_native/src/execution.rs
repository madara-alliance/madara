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

    // Cache miss - record metric and proceed to compilation
    super::metrics::metrics().record_cache_miss();

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
    if let Some(cached) = cache::try_get_from_memory_cache(class_hash) {
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
        Ok(Some(cached)) => {
            Some(cached)
        }
        Ok(None) => {
            // Cache miss - will fall through to compilation
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
            super::metrics::metrics().record_compilation_blocking_conversion_time(conversion_elapsed.as_millis() as u64);
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
