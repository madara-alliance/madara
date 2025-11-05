//! Execution flow for Cairo Native classes
//!
//! This module handles the main execution flow for Sierra classes with native execution support.

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use std::sync::Arc;
use std::time::Instant;

use super::config;
use crate::SierraConvertedClass;

use super::cache;
use super::compilation;

/// Handle Sierra class conversion with native execution support.
///
/// Implements the full caching and compilation flow for Sierra classes.
pub(crate) fn handle_sierra_class(
    sierra: &SierraConvertedClass,
    class_hash: &starknet_types_core::felt::Felt,
    compiled: &Arc<crate::CompiledSierra>,
    info: &crate::SierraClassInfo,
) -> Result<RunnableCompiledClass, ProgramError> {
    // Runtime check: is native execution enabled?
    let config = config::get_config();
    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        native_enabled = config.enable_native_execution,
        "handle_sierra_class"
    );

    if !config.enable_native_execution {
        // Native execution disabled at runtime - use Cairo VM
        tracing::debug!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "disabled_fallback_to_vm"
        );
        let sierra_version = info.contract_class.sierra_version().map_err(|e| {
            tracing::error!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                "sierra_version_error"
            );
            ProgramError::Parse(serde::de::Error::custom("Failed to get sierra version from program"))
        })?;
        // Convert CompiledSierra directly to the type expected by ApiContractClass::V1
        let vm_class = RunnableCompiledClass::try_from(ApiContractClass::V1((
            compiled.as_ref().try_into().map_err(|e| {
                tracing::error!(
                    target: "madara.cairo_native",
                    class_hash = %format!("{:#x}", class_hash),
                    error = %format!("{:?}", e),
                    "vm_conversion_error"
                );
                ProgramError::Parse(serde::de::Error::custom(format!("Failed to convert to VM class: {:?}", e)))
            })?,
            sierra_version,
        )))?;
        tracing::debug!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "returning_vm_class"
        );
        return Ok(vm_class);
    }

    // Native execution enabled - proceed with native path
    let start = Instant::now();

    // Check in-memory cache first
    if let Some(cached) = cache::try_get_from_memory_cache(class_hash) {
        let elapsed = start.elapsed();
        tracing::info!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            elapsed = ?elapsed,
            "native_from_memory"
        );
        return Ok(cached);
    }

    // Try to load from disk cache with timeout protection
    match cache::try_get_from_disk_cache(class_hash, sierra) {
        Ok(Some(cached)) => {
            let elapsed = start.elapsed();
            tracing::info!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                elapsed = ?elapsed,
                "native_from_disk"
            );
            return Ok(cached);
        }
        Ok(None) => {
            // Disk cache miss or timeout - will fall through to compilation/VM fallback
        }
        Err(e) => {
            // Error loading from disk - log and fall through to fallback
            let elapsed = start.elapsed();
            tracing::warn!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %e,
                elapsed = ?elapsed,
                "disk_cache_error_fallback"
            );
        }
    }

    // Not in cache and not on disk - check compilation mode
    super::metrics::metrics().record_cache_miss();

    let config = config::get_config();

    // Check if blocking mode is enabled
    if config.is_blocking_mode() {
        // BLOCKING MODE: Compile synchronously and panic if compilation fails
        tracing::info!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "compilation_blocking_miss"
        );

        match compilation::compile_native_blocking(*class_hash, sierra, None) {
            Ok(native_class) => {
                tracing::info!(
                    target: "madara.cairo_native",
                    class_hash = %format!("{:#x}", class_hash),
                    "compilation_blocking_complete"
                );
                let runnable = RunnableCompiledClass::from(native_class.as_ref().clone());
                tracing::debug!(
                    target: "madara.cairo_native",
                    class_hash = %format!("{:#x}", class_hash),
                    "returning_native_blocking"
                );
                return Ok(runnable);
            }
            Err(e) => {
                // In blocking mode, failures are fatal - return error (will panic upstream)
                tracing::error!(
                    target: "madara.cairo_native",
                    class_hash = %format!("{:#x}", class_hash),
                    error = %e,
                    "compilation_blocking_failed"
                );
                // Metrics are already recorded in compile_native_blocking
                return Err(e.into());
            }
        }
    }

    // ASYNC MODE: Spawn background compilation and fall back to VM
    // Check if this class previously failed compilation - if so, retry
    let should_retry = compilation::FAILED_COMPILATIONS.remove(class_hash).is_some();
    if should_retry {
        tracing::info!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            "compilation_retrying_previously_failed"
        );
    }

    let is_already_compiling = compilation::COMPILATION_IN_PROGRESS.contains_key(class_hash);
    if !is_already_compiling {
        // Get compilation stats for logging
        let in_progress_count = compilation::COMPILATION_IN_PROGRESS.len();
        let max_concurrent = config::get_max_concurrent_compilations();

        tracing::debug!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            in_progress = in_progress_count,
            max_concurrent = max_concurrent,
            "compilation_async_spawning"
        );
        compilation::spawn_native_compilation(*class_hash, Arc::new(sierra.clone()));
    }

    // Fall back to cairo-vm (CASM execution) while compilation happens in background
    super::metrics::metrics().record_vm_fallback();
    tracing::info!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "fallback_to_vm_async"
    );
    let sierra_version = info.contract_class.sierra_version().map_err(|e| {
        tracing::error!(
            target: "madara.cairo_native",
            class_hash = %format!("{:#x}", class_hash),
            error = %e,
            "async_sierra_version_error"
        );
        ProgramError::Parse(serde::de::Error::custom("Failed to get sierra version from program"))
    })?;
    // Convert CompiledSierra directly to the type expected by ApiContractClass::V1
    let vm_class = RunnableCompiledClass::try_from(ApiContractClass::V1((
        compiled.as_ref().try_into().map_err(|e| {
            tracing::error!(
                target: "madara.cairo_native",
                class_hash = %format!("{:#x}", class_hash),
                error = %format!("{:?}", e),
                "async_vm_conversion_error"
            );
            ProgramError::Parse(serde::de::Error::custom(format!("Failed to convert to VM class: {:?}", e)))
        })?,
        sierra_version,
    )))?;
    tracing::debug!(
        target: "madara.cairo_native",
        class_hash = %format!("{:#x}", class_hash),
        "returning_vm_class_async"
    );
    Ok(vm_class)
}
