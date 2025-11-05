//! Conversion to Blockifier's `RunnableCompiledClass` for execution
//!
//! This module provides the main entry point for converting `ConvertedClass` instances
//! to Blockifier's `RunnableCompiledClass` for execution. It dispatches to either:
//! - Cairo Native execution (if enabled and available)
//! - Cairo VM execution (fallback or default)

use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use starknet_api::contract_class::ContractClass as ApiContractClass;

use crate::{ConvertedClass, LegacyConvertedClass};

use crate::native;

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = ProgramError;

    /// Convert a `ConvertedClass` to a `RunnableCompiledClass` for execution.
    ///
    /// This is the main entry point for class execution. It routes to:
    /// - **Legacy classes**: Always uses Cairo VM execution
    /// - **Sierra classes**: Uses Cairo Native execution if enabled, otherwise Cairo VM
    ///
    /// See the `native` module for detailed architecture and execution flow.
    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => {
                RunnableCompiledClass::try_from(ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi()?))
            }

            ConvertedClass::Sierra(sierra @ crate::SierraConvertedClass { class_hash, compiled, info }) => {
                native::execution::handle_sierra_class(sierra, class_hash, compiled, info)
            }
        }
    }
}
