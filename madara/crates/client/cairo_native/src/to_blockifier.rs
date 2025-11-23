//! Conversion to Blockifier's `RunnableCompiledClass` for execution
//!
//! This module provides the main entry point for converting `ConvertedClass` instances
//! to Blockifier's `RunnableCompiledClass` for execution. It dispatches to either:
//! - Cairo Native execution (if enabled and available)
//! - Cairo VM execution (fallback or default)

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use std::sync::Arc;

use mp_class::{ConvertedClass, LegacyConvertedClass, SierraConvertedClass};

use crate::config::NativeConfig;
use crate::execution;

/// Convert a `ConvertedClass` to a `RunnableCompiledClass` for execution.
///
/// This is the preferred way to convert classes as it accepts config as a parameter
/// instead of relying on thread-local storage. Config is required and must be passed from the caller.
///
/// Routes to:
/// - **Legacy classes**: Always uses Cairo VM execution
/// - **Sierra classes**: Uses Cairo Native execution if enabled in config, otherwise Cairo VM
///
/// # Panics
///
/// Panics if `cairo_native_config` is `None`. Config must always be provided.
pub fn convert_to_runnable(
    converted_class: &ConvertedClass,
    cairo_native_config: &Arc<NativeConfig>,
) -> Result<RunnableCompiledClass, ProgramError> {
    match converted_class {
        ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => {
            RunnableCompiledClass::try_from(ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi()?))
        }

        ConvertedClass::Sierra(sierra @ SierraConvertedClass { class_hash, compiled, info }) => {
            execution::handle_sierra_class(sierra, class_hash, compiled, info, cairo_native_config.clone())
        }
    }
}
