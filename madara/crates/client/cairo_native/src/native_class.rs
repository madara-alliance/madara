//! Native compiled class wrapper
//!
//! This module provides a wrapper around `NativeCompiledClassV1` to improve encapsulation
//! and make refactoring safer. The wrapper implements `Deref` to provide transparent access
//! to the inner type while adding type safety and better organization.

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::execution::native::contract_class::NativeCompiledClassV1;
use std::ops::Deref;
use std::sync::Arc;

/// Wrapper around `Arc<NativeCompiledClassV1>` for improved encapsulation.
///
/// This wrapper provides a type-safe way to work with native compiled classes
/// and makes refactoring safer by centralizing conversion methods.
#[derive(Debug, Clone)]
pub struct NativeCompiledClass(Arc<NativeCompiledClassV1>);

impl NativeCompiledClass {
    /// Create a new `NativeCompiledClass` from an executor and blockifier compiled class.
    ///
    /// This wraps the creation of `NativeCompiledClassV1` and provides a consistent API.
    pub fn new(
        executor: cairo_native::executor::AotContractExecutor,
        blockifier_compiled_class: blockifier::execution::contract_class::CompiledClassV1,
    ) -> Self {
        Self(Arc::new(NativeCompiledClassV1::new(executor, blockifier_compiled_class)))
    }

    /// Create a new `NativeCompiledClass` from an existing `Arc<NativeCompiledClassV1>`.
    ///
    /// This is useful when converting from existing code that uses `Arc<NativeCompiledClassV1>`.
    pub fn from_arc(arc: Arc<NativeCompiledClassV1>) -> Self {
        Self(arc)
    }

    /// Convert this native compiled class to a `RunnableCompiledClass` for execution.
    ///
    /// This is the main conversion method used throughout the codebase to convert
    /// native compiled classes to the format expected by Blockifier for execution.
    pub fn to_runnable(&self) -> RunnableCompiledClass {
        RunnableCompiledClass::from(self.0.as_ref().clone())
    }

    /// Get a reference to the inner `Arc<NativeCompiledClassV1>`.
    ///
    /// This is useful when you need to pass the inner type to functions that
    /// haven't been updated to use the wrapper yet.
    pub fn as_inner(&self) -> &Arc<NativeCompiledClassV1> {
        &self.0
    }
}

impl Deref for NativeCompiledClass {
    type Target = NativeCompiledClassV1;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Arc<NativeCompiledClassV1>> for NativeCompiledClass {
    fn from(arc: Arc<NativeCompiledClassV1>) -> Self {
        Self::from_arc(arc)
    }
}

impl From<NativeCompiledClassV1> for NativeCompiledClass {
    fn from(class: NativeCompiledClassV1) -> Self {
        Self(Arc::new(class))
    }
}
