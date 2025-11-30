//! Shared test utilities for Cairo Native tests
//!
//! This module provides common test helpers used across cache.rs, execution.rs, and compilation.rs
//! to reduce code duplication and improve maintainability.

#[cfg(test)]
use mp_class::SierraConvertedClass;
#[cfg(test)]
use rstest::fixture;
#[cfg(test)]
use starknet_api::core::ClassHash;
#[cfg(test)]
use starknet_types_core::felt::Felt;
#[cfg(test)]
use std::sync::{atomic::AtomicU64, Arc, OnceLock};
#[cfg(test)]
use tempfile::TempDir;

/// Cached Sierra class compiled once and reused across all tests.
/// Uses OnceLock to ensure it's compiled only once.
#[cfg(test)]
static CACHED_SIERRA_CLASS: OnceLock<SierraConvertedClass> = OnceLock::new();

/// Compile the test Sierra class once and cache it for reuse.
/// This is called once, and subsequent calls return the cached result.
#[cfg(test)]
pub fn get_test_sierra_class() -> &'static SierraConvertedClass {
    CACHED_SIERRA_CLASS.get_or_init(|| {
        use m_cairo_test_contracts::TEST_CONTRACT_SIERRA;
        use mp_class::{CompiledSierra, FlattenedSierraClass, SierraClassInfo};
        use serde_json::Value;
        use std::sync::Arc;

        // Parse as JSON value first to check the structure
        let mut json_value: Value =
            serde_json::from_slice(TEST_CONTRACT_SIERRA).expect("Failed to parse TEST_CONTRACT_SIERRA as JSON");

        // Handle abi field - convert from object/array to string if needed
        if let Some(abi_value) = json_value.get_mut("abi") {
            if !abi_value.is_string() {
                *abi_value = Value::String(serde_json::to_string(abi_value).expect("Failed to serialize abi"));
            }
        }

        // Check if sierra_program is an array (flattened) or string (compressed)
        let flattened_sierra = if let Some(sierra_program) = json_value.get("sierra_program") {
            if sierra_program.is_array() {
                serde_json::from_value::<FlattenedSierraClass>(json_value)
                    .expect("Failed to parse as FlattenedSierraClass")
            } else if sierra_program.is_string() {
                use mp_class::CompressedSierraClass;
                let compressed = serde_json::from_value::<CompressedSierraClass>(json_value)
                    .expect("Failed to parse as CompressedSierraClass");
                FlattenedSierraClass::try_from(compressed).expect("Failed to decompress CompressedSierraClass")
            } else {
                panic!("sierra_program field is neither an array nor a string");
            }
        } else {
            panic!("JSON does not contain sierra_program field");
        };

        // Compile to CASM to get compiled class hash
        let (compiled_class_hash, casm_class) =
            flattened_sierra.compile_to_casm().expect("Failed to compile Sierra to CASM");
        let compiled_sierra = CompiledSierra::try_from(&casm_class).expect("Failed to convert CASM to CompiledSierra");

        // Create SierraClassInfo
        let sierra_info = SierraClassInfo {
            contract_class: Arc::new(flattened_sierra),
            compiled_class_hash,
            compiled_class_hash_v2: None,
        };

        // Create SierraConvertedClass
        SierraConvertedClass { class_hash: compiled_class_hash, info: sierra_info, compiled: Arc::new(compiled_sierra) }
    })
}

/// Module-specific base values for unique class hash generation.
/// This ensures no collisions between different test modules.
#[cfg(test)]
const MODULE_BASES: [&str; 4] = [
    "0x0000000000000000000000000000000000000000000000000000000000000000", // Reserved/unused
    "0x1000000000000000000000000000000000000000000000000000000000000000", // cache.rs (module_id=1)
    "0x2000000000000000000000000000000000000000000000000000000000000000", // execution.rs (module_id=2)
    "0x3000000000000000000000000000000000000000000000000000000000000000", // compilation.rs (module_id=3)
];

/// Global counter for generating unique test class hashes.
/// Shared across all modules but each module uses a different base value.
#[cfg(test)]
static TEST_CLASS_HASH_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Create a unique test class hash for a specific module.
///
/// # Arguments
/// * `module_id` - Module identifier: 1=cache, 2=execution, 3=compilation
///
/// # Panics
/// Panics if `module_id` is out of range (must be 1-3).
#[cfg(test)]
pub fn create_unique_test_class_hash(module_id: u8) -> ClassHash {
    if module_id == 0 || module_id as usize >= MODULE_BASES.len() {
        panic!("Invalid module_id: {}. Must be between 1 and {}", module_id, MODULE_BASES.len() - 1);
    }

    let counter = TEST_CLASS_HASH_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = Felt::from_hex_unchecked(MODULE_BASES[module_id as usize]);
    ClassHash(base + Felt::from(counter))
}

/// Create a test configuration for Cairo Native tests.
///
/// # Arguments
/// * `temp_dir` - Temporary directory for cache files (already exists, created by TempDir::new())
/// * `mode` - Optional compilation mode (defaults to Async if None)
/// * `init_semaphore` - Whether to initialize compilation semaphore (defaults to true)
///
/// # Returns
/// Returns `config::NativeConfig`.
#[cfg(test)]
pub fn create_test_config(
    temp_dir: &TempDir,
    mode: Option<crate::config::NativeCompilationMode>,
    init_semaphore: bool,
) -> crate::config::NativeConfig {
    use crate::config;
    use std::time::Duration;

    // temp_dir.path() already exists (created by TempDir::new())
    let builder = config::NativeConfig::builder()
        .with_cache_dir(temp_dir.path().to_path_buf())
        .with_compilation_mode(mode.unwrap_or(config::NativeCompilationMode::Async))
        .with_compilation_timeout(Duration::from_secs(30));

    if init_semaphore {
        // Extract max_concurrent_compilations from builder before building
        let max_concurrent = builder.max_concurrent_compilations();
        crate::compilation::init_compilation_semaphore(max_concurrent);
    }

    builder.build()
}

/// Create a test configuration as Arc (convenience wrapper).
#[cfg(test)]
pub fn create_test_config_arc(
    temp_dir: &TempDir,
    mode: Option<crate::config::NativeCompilationMode>,
    init_semaphore: bool,
) -> std::sync::Arc<crate::config::NativeConfig> {
    std::sync::Arc::new(create_test_config(temp_dir, mode, init_semaphore))
}

/// Internal helper to create a native compiled class from Sierra.
/// This is the common logic shared across all native class creation functions.
///
/// # Arguments
/// * `sierra` - The Sierra class to compile
/// * `so_path` - Path where the compiled .so file should be saved
///
/// # Returns
/// Returns `NativeCompiledClass` on success.
#[cfg(test)]
pub fn create_native_class_internal(
    sierra: &SierraConvertedClass,
    so_path: &std::path::Path,
) -> Result<crate::native_class::NativeCompiledClass, Box<dyn std::error::Error>> {
    // Compile Sierra to native
    let executor = sierra.info.contract_class.compile_to_native(so_path)?;

    // Convert Sierra to blockifier compiled class
    let blockifier_compiled_class = crate::compilation::convert_sierra_to_blockifier_class(sierra)?;

    // Create NativeCompiledClass
    let native_class = crate::native_class::NativeCompiledClass::new(executor, blockifier_compiled_class);

    Ok(native_class)
}

/// Fixture: Provides a compiled Sierra class (compiled once, reused)
/// Use with rstest: `#[rstest] fn test_name(sierra_class: SierraConvertedClass) { ... }`
#[cfg(test)]
#[fixture]
pub fn sierra_class() -> SierraConvertedClass {
    get_test_sierra_class().clone()
}

/// Fixture: Provides a temporary directory for test cache files
/// Use with rstest: `#[rstest] fn test_name(temp_dir: TempDir) { ... }`
#[cfg(test)]
#[fixture]
pub fn temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temp dir")
}

/// Fixture: Provides a simple test config with native execution enabled
/// Use with rstest: `#[rstest] fn test_name(temp_dir: TempDir, test_config: config::NativeConfig) { ... }`
#[cfg(test)]
#[fixture]
pub fn test_config(temp_dir: TempDir) -> crate::config::NativeConfig {
    create_test_config(&temp_dir, None, false)
}

/// Fixture: Provides a test config as Arc with async mode
/// Use with rstest: `#[rstest] fn test_name(temp_dir: TempDir, async_config: Arc<config::NativeConfig>) { ... }`
#[cfg(test)]
#[fixture]
pub fn async_config(temp_dir: TempDir) -> Arc<crate::config::NativeConfig> {
    create_test_config_arc(&temp_dir, Some(crate::config::NativeCompilationMode::Async), true)
}

/// Fixture: Provides a test config as Arc with blocking mode
/// Use with rstest: `#[rstest] fn test_name(temp_dir: TempDir, blocking_config: Arc<config::NativeConfig>) { ... }`
#[cfg(test)]
#[fixture]
pub fn blocking_config(temp_dir: TempDir) -> Arc<crate::config::NativeConfig> {
    create_test_config_arc(&temp_dir, Some(crate::config::NativeCompilationMode::Blocking), true)
}

/// Macro to assert multiple counter values in tests.
///
/// Usage:
/// ```rust
/// assert_counters!(
///     CACHE_HITS_MEMORY: 1,
///     CACHE_MEMORY_MISS: 0,
///     CACHE_HITS_DISK: 0,
/// );
/// ```
#[cfg(test)]
#[macro_export]
macro_rules! assert_counters {
    (
        $($counter:ident: $expected:expr),* $(,)?
    ) => {
        $(
            assert_eq!(
                $crate::metrics::test_counters::$counter.load(std::sync::atomic::Ordering::Relaxed),
                $expected,
                "Counter {} should be {}",
                stringify!($counter),
                $expected
            );
        )*
    };
}

/// Assert that a RunnableCompiledClass is a VM class (not Native).
///
/// This helper function is used in tests to verify that a `RunnableCompiledClass`
/// is a VM-compiled class (V0 or V1) and not a Native-compiled class (V1Native).
#[cfg(test)]
pub fn assert_is_vm_class(runnable: &blockifier::execution::contract_class::RunnableCompiledClass) {
    match runnable {
        blockifier::execution::contract_class::RunnableCompiledClass::V0(_) => {} // VM class - OK
        blockifier::execution::contract_class::RunnableCompiledClass::V1(_) => {} // VM class - OK
        blockifier::execution::contract_class::RunnableCompiledClass::V1Native(_) => {
            panic!("Expected VM class but got Native class");
        }
    }
}

/// Assert that a RunnableCompiledClass is a Native class (not VM).
///
/// This helper function is used in tests to verify that a `RunnableCompiledClass`
/// is a Native-compiled class (V1Native) and not a VM-compiled class (V0 or V1).
#[cfg(test)]
pub fn assert_is_native_class(runnable: &blockifier::execution::contract_class::RunnableCompiledClass) {
    match runnable {
        blockifier::execution::contract_class::RunnableCompiledClass::V1Native(_) => {} // Native class - OK
        blockifier::execution::contract_class::RunnableCompiledClass::V0(_) => {
            panic!("Expected Native class but got VM class");
        }
        blockifier::execution::contract_class::RunnableCompiledClass::V1(_) => {
            panic!("Expected Native class but got VM class");
        }
    }
}
