//! Configuration for Cairo Native compilation and caching
//!
//! This module provides configuration management for Cairo Native execution,
//! including compilation settings, cache limits, and execution modes.
//!
//! # Overview
//!
//! The configuration is created from CLI parameters and passed through function parameters
//! (no global state). This ensures consistent behavior and makes parallel testing easier.
//!
//! # Configuration Sources
//!
//! Configuration can be provided via:
//! 1. CLI arguments (see `CairoNativeParams` in `node/src/cli/cairo_native.rs`)
//! 2. Environment variables (e.g., `MADARA_ENABLE_NATIVE_EXECUTION`)
//! 3. Programmatic configuration (for testing)
//!
//! # Default Behavior
//!
//! By default, native execution is **disabled** (`enable_native_execution: false`).
//! This matches the CLI default and ensures backward compatibility. All contracts
//! will use Cairo VM execution when native is disabled.
//!
//! # Important Constants
//!
//! - `DEFAULT_MEMORY_CACHE_SIZE`: 1000 classes
//! - `DEFAULT_DISK_CACHE_SIZE_BYTES`: 10 GB
//! - `DEFAULT_COMPILATION_TIMEOUT_SECS`: 300 seconds (5 minutes)
//! - `DEFAULT_MAX_CONCURRENT_COMPILATIONS`: 4

use std::path::PathBuf;
use std::time::Duration;

/// Default disk cache size: 10 GB
pub const DEFAULT_DISK_CACHE_SIZE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Default compilation timeout: 5 minutes
pub const DEFAULT_COMPILATION_TIMEOUT_SECS: u64 = 300;

/// Default memory cache lookup timeout: 100 milliseconds
pub const DEFAULT_MEMORY_CACHE_TIMEOUT_MS: u64 = 100;

/// Default disk cache load timeout: 2 seconds
pub const DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS: u64 = 2;

/// Default polling interval for waiting on compilation completion in blocking mode: 50 milliseconds
pub const DEFAULT_COMPILATION_POLL_INTERVAL_MS: u64 = 50;

/// Default memory cache size: 1000 classes
pub const DEFAULT_MEMORY_CACHE_SIZE: usize = 1000;

/// Default maximum concurrent compilations
pub const DEFAULT_MAX_CONCURRENT_COMPILATIONS: usize = 4;

/// Default maximum number of failed compilation entries to keep before evicting oldest.
/// This prevents unbounded growth while still allowing retry for recently failed classes.
pub const DEFAULT_MAX_FAILED_COMPILATIONS: usize = 10_000;

/// Default cache directory path for compiled native classes
pub const DEFAULT_CACHE_DIR: &str = "/usr/share/madara/data/classes";

/// Native compilation mode determines how compilation failures are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCompilationMode {
    /// Compile in background, use Cairo VM as fallback (production mode).
    ///
    /// In this mode:
    /// - Compilation happens asynchronously in the background
    /// - VM fallback is used immediately while compilation runs
    /// - Failures are logged and retried on next request
    /// - Best for production where availability is critical
    Async,

    /// Wait for compilation, fail if compilation fails (testing/debugging mode).
    ///
    /// In this mode:
    /// - Compilation happens synchronously, blocking until complete
    /// - No VM fallback - failures cause transaction errors
    /// - Best for testing or when strict native execution is required
    Blocking,
}

/// Configuration for Cairo Native execution when enabled.
///
/// This struct holds all runtime configuration for native execution.
/// It is only used when `NativeConfig::Enabled` variant is selected.
#[derive(Debug, Clone)]
pub struct NativeExecutionConfig {
    /// Directory path for storing compiled native classes
    pub cache_dir: PathBuf,

    /// Maximum number of classes to keep in memory cache (None = unlimited)
    pub max_memory_cache_size: Option<usize>,

    /// Maximum disk cache size in bytes (None = unlimited)
    pub max_disk_cache_size: Option<u64>,

    /// Maximum number of concurrent compilations
    pub max_concurrent_compilations: usize,

    /// Maximum time to wait for a single compilation
    pub compilation_timeout: Duration,

    /// Maximum time to wait for memory cache lookup
    pub memory_cache_timeout: Duration,

    /// Maximum time to wait for disk cache load
    pub disk_cache_load_timeout: Duration,

    /// Compilation mode (async or blocking)
    pub compilation_mode: NativeCompilationMode,

    /// Enable automatic retry of failed compilations.
    ///
    /// When `true`, classes that failed compilation in async mode are automatically retried
    /// on the next request. When `false`, failed compilations are not retried automatically.
    pub enable_retry: bool,

    /// Maximum number of failed compilation entries to keep before evicting oldest.
    ///
    /// When a compilation fails, the class hash is stored in `FAILED_COMPILATIONS` for retry tracking.
    /// This setting limits how many failed entries are kept in memory. When the limit is reached,
    /// the oldest entries are evicted using LRU policy.
    ///
    /// Higher values allow more failed classes to be tracked for retry, but use more memory.
    pub max_failed_compilations: usize,
}

/// Configuration for Cairo Native execution and compilation.
///
/// This enum provides type-safe configuration that ensures compile-time guarantees.
/// When `Disabled`, all contracts use Cairo VM regardless of cache state.
/// When `Enabled`, native execution is used with the provided configuration.
///
/// This design prevents accidental access to execution config when native is disabled,
/// providing better type safety than a boolean flag.
#[derive(Debug, Clone)]
pub enum NativeConfig {
    /// Native execution is disabled - all contracts use Cairo VM.
    Disabled,

    /// Native execution is enabled with the provided configuration.
    Enabled(NativeExecutionConfig),
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self::Disabled
    }
}

impl Default for NativeExecutionConfig {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR), // Use default path, actual path comes from CLI/config
            max_memory_cache_size: Some(DEFAULT_MEMORY_CACHE_SIZE),
            max_disk_cache_size: Some(DEFAULT_DISK_CACHE_SIZE_BYTES),
            max_concurrent_compilations: DEFAULT_MAX_CONCURRENT_COMPILATIONS,
            compilation_timeout: Duration::from_secs(DEFAULT_COMPILATION_TIMEOUT_SECS),
            memory_cache_timeout: Duration::from_millis(DEFAULT_MEMORY_CACHE_TIMEOUT_MS),
            disk_cache_load_timeout: Duration::from_secs(DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS),
            compilation_mode: NativeCompilationMode::Async, // Default to async
            enable_retry: true,                             // Retry enabled by default
            max_failed_compilations: DEFAULT_MAX_FAILED_COMPILATIONS,
        }
    }
}

/// Builder for `NativeConfig` that can build both `Disabled` and `Enabled` variants.
///
/// This builder uses a self-consuming builder pattern and can build either variant.
/// By default, the builder starts disabled. Use `.enable()` or any `.with_*()` method
/// to enable native execution with configuration.
///
/// # Example
///
/// ```rust
/// use mc_class_exec::config::{NativeConfig, NativeCompilationMode};
/// use std::path::PathBuf;
///
/// // Build enabled config
/// let enabled = NativeConfig::builder()
///     .with_cache_dir(PathBuf::from("/tmp/cache"))
///     .with_max_memory_cache_size(Some(2000))
///     .with_compilation_mode(NativeCompilationMode::Blocking)
///     .build();
///
/// // Build disabled config
/// let disabled = NativeConfig::builder()
///     .disable()
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct NativeConfigBuilder {
    config: Option<NativeExecutionConfig>,
}

impl NativeConfigBuilder {
    /// Create a new builder (disabled by default).
    pub fn new() -> Self {
        Self { config: None }
    }

    /// Enable native execution with default configuration.
    pub fn enable(mut self) -> Self {
        self.config = Some(NativeExecutionConfig::default());
        self
    }

    /// Disable native execution (explicit).
    pub fn disable(mut self) -> Self {
        self.config = None;
        self
    }

    /// Build the final immutable configuration.
    ///
    /// Returns `NativeConfig::Disabled` if disabled, `NativeConfig::Enabled(config)` if enabled.
    pub fn build(self) -> NativeConfig {
        match self.config {
            None => NativeConfig::Disabled,
            Some(config) => NativeConfig::Enabled(config),
        }
    }

    /// Get the max_concurrent_compilations value from the builder (for initialization purposes).
    ///
    /// Returns the default value if disabled.
    pub fn max_concurrent_compilations(&self) -> usize {
        self.config.as_ref().map(|c| c.max_concurrent_compilations).unwrap_or(DEFAULT_MAX_CONCURRENT_COMPILATIONS)
    }

    /// Ensure the builder is enabled, initializing with defaults if needed.
    fn ensure_enabled(&mut self) {
        if self.config.is_none() {
            self.config = Some(NativeExecutionConfig::default());
        }
    }

    /// Set the cache directory (automatically enables native execution).
    pub fn with_cache_dir(mut self, path: PathBuf) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.cache_dir = path;
        }
        self
    }

    /// Set the maximum memory cache size (None = unlimited).
    /// Automatically enables native execution if disabled.
    pub fn with_max_memory_cache_size(mut self, size: Option<usize>) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.max_memory_cache_size = size;
        }
        self
    }

    /// Set the maximum disk cache size in bytes (None = unlimited).
    /// Automatically enables native execution if disabled.
    pub fn with_max_disk_cache_size(mut self, size: Option<u64>) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.max_disk_cache_size = size;
        }
        self
    }

    /// Set the maximum number of concurrent compilations.
    /// Automatically enables native execution if disabled.
    pub fn with_max_concurrent_compilations(mut self, max: usize) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.max_concurrent_compilations = max;
        }
        self
    }

    /// Set the compilation timeout.
    /// Automatically enables native execution if disabled.
    pub fn with_compilation_timeout(mut self, timeout: Duration) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.compilation_timeout = timeout;
        }
        self
    }

    /// Set the memory cache lookup timeout.
    /// Automatically enables native execution if disabled.
    pub fn with_memory_cache_timeout(mut self, timeout: Duration) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.memory_cache_timeout = timeout;
        }
        self
    }

    /// Set the disk cache load timeout.
    /// Automatically enables native execution if disabled.
    pub fn with_disk_cache_load_timeout(mut self, timeout: Duration) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.disk_cache_load_timeout = timeout;
        }
        self
    }

    /// Set the compilation mode.
    /// Automatically enables native execution if disabled.
    pub fn with_compilation_mode(mut self, mode: NativeCompilationMode) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.compilation_mode = mode;
        }
        self
    }

    /// Enable or disable automatic retry of failed compilations.
    /// Automatically enables native execution if disabled.
    pub fn with_enable_retry(mut self, enabled: bool) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.enable_retry = enabled;
        }
        self
    }

    /// Set the maximum number of failed compilation entries to keep.
    /// Automatically enables native execution if disabled.
    pub fn with_max_failed_compilations(mut self, max: usize) -> Self {
        self.ensure_enabled();
        if let Some(ref mut config) = self.config {
            config.max_failed_compilations = max;
        }
        self
    }
}

impl Default for NativeConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeConfig {
    /// Create a new configuration with default values (disabled).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for native execution configuration.
    ///
    /// The builder starts disabled by default. Use `.enable()` or any `.with_*()` method
    /// to enable native execution with configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use mc_class_exec::config::{NativeConfig, NativeCompilationMode};
    /// use std::path::PathBuf;
    ///
    /// // Build enabled config
    /// let enabled = NativeConfig::builder()
    ///     .with_cache_dir(PathBuf::from("/tmp/cache"))
    ///     .with_max_memory_cache_size(Some(2000))
    ///     .with_compilation_mode(NativeCompilationMode::Blocking)
    ///     .build();
    ///
    /// // Build disabled config
    /// let disabled = NativeConfig::builder()
    ///     .disable()
    ///     .build();
    /// ```
    pub fn builder() -> NativeConfigBuilder {
        NativeConfigBuilder::new()
    }

    /// Enable native execution with default configuration.
    pub fn enable(self) -> Self {
        Self::Enabled(NativeExecutionConfig::default())
    }

    /// Disable native execution.
    pub fn disable(self) -> Self {
        Self::Disabled
    }

    /// Enable or disable native execution (runtime control).
    ///
    /// If enabling and currently disabled, creates a new Enabled config with defaults.
    /// If disabling, converts to Disabled.
    pub fn with_native_execution(self, enabled: bool) -> Self {
        if enabled {
            match self {
                Self::Disabled => Self::Enabled(NativeExecutionConfig::default()),
                Self::Enabled(_) => self, // Already enabled, keep existing config
            }
        } else {
            Self::Disabled
        }
    }

    /// Get a reference to the execution config.
    /// Returns None if disabled.
    pub fn execution_config(&self) -> Option<&NativeExecutionConfig> {
        match self {
            Self::Disabled => None,
            Self::Enabled(ref config) => Some(config),
        }
    }

    /// Check if blocking mode is enabled.
    /// Returns false if native execution is disabled.
    pub fn is_blocking_mode(&self) -> bool {
        match self {
            Self::Disabled => false,
            Self::Enabled(config) => matches!(config.compilation_mode, NativeCompilationMode::Blocking),
        }
    }

    /// Check if native execution is enabled.
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Skip all validation if native execution is disabled
        let config = match self {
            Self::Disabled => return Ok(()),
            Self::Enabled(config) => config,
        };

        // Native execution is enabled - validate all parameters
        if config.max_concurrent_compilations == 0 {
            return Err("max_concurrent_compilations must be greater than 0".to_string());
        }

        // Validate cache limits if set (must be > 0 if Some)
        if let Some(size) = config.max_memory_cache_size {
            if size == 0 {
                return Err("max_memory_cache_size must be greater than 0 if set".to_string());
            }
        }
        if let Some(size) = config.max_disk_cache_size {
            if size == 0 {
                return Err("max_disk_cache_size must be greater than 0 if set".to_string());
            }
        }

        if config.compilation_timeout.as_secs() == 0 {
            return Err("compilation_timeout must be greater than 0".to_string());
        }

        // Try to create cache directory if it doesn't exist
        if !config.cache_dir.exists() {
            std::fs::create_dir_all(&config.cache_dir)
                .map_err(|e| format!("Failed to create cache directory {:?}: {}", config.cache_dir, e))?;
        }

        // Check if directory is writable
        if !config.cache_dir.is_dir() {
            return Err(format!("Cache path {:?} is not a directory", config.cache_dir));
        }

        Ok(())
    }
}

/// Setup Cairo Native configuration: validate, initialize semaphore, register metrics, and log status.
///
/// This is a convenience function that handles all the setup steps needed when
/// initializing Cairo Native configuration at startup. It:
/// 1. Validates the configuration
/// 2. Initializes the compilation semaphore with the configured concurrency limit
/// 3. Registers OTEL metrics (if OTEL is initialized)
/// 4. Logs the configuration status
///
/// # Errors
///
/// Returns an error if configuration validation fails.
pub fn setup_and_log(config: &NativeConfig) -> Result<(), String> {
    // Validate the configuration
    config.validate()?;

    // Register OTEL metrics (will work even if OTEL isn't initialized yet, but won't export)
    // Metrics are registered lazily on first access, but we register here to ensure
    // they're available as soon as OTEL is initialized
    let _ = crate::metrics::metrics(); // Force lazy initialization

    // Log configuration and initialize resources only if enabled
    match config {
        NativeConfig::Disabled => {
            tracing::info!("Cairo native disabled - all contracts will use Cairo VM");
        }
        NativeConfig::Enabled(exec_config) => {
            // Initialize the compilation semaphore with the configured concurrency limit
            // Only needed when native execution is enabled
            crate::compilation::init_compilation_semaphore(exec_config.max_concurrent_compilations);
            // Log cache directory initialization
            tracing::info!("💾 Cairo-native directory initialized: {}", exec_config.cache_dir.display());

            let mode_description = match exec_config.compilation_mode {
                NativeCompilationMode::Async => "Async (VM fallback enabled)",
                NativeCompilationMode::Blocking => "Blocking (strictly native, no VM fallback)",
            };
            let memory_cache_desc = match exec_config.max_memory_cache_size {
                Some(size) => format!("{} classes", size),
                None => "unlimited".to_string(),
            };
            let disk_cache_desc = match exec_config.max_disk_cache_size {
                Some(size) => {
                    let gb = size as f64 / (1024.0 * 1024.0 * 1024.0);
                    format!("{:.1} GB", gb)
                }
                None => "unlimited".to_string(),
            };
            tracing::info!(
                "🚀 Cairo native enabled - mode: {}, memory_cache: {}, disk_cache: {}, max_concurrent: {}",
                mode_description,
                memory_cache_desc,
                disk_cache_desc,
                exec_config.max_concurrent_compilations
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        // Test default config is disabled
        let config = NativeConfig::default();
        assert!(!config.is_enabled());
        assert!(matches!(config, NativeConfig::Disabled));

        // Test default enabled config with actual default values
        let enabled_config = NativeConfig::builder()
            .enable() // Explicitly enable to get defaults
            .build();
        if let NativeConfig::Enabled(exec_config) = enabled_config {
            // Verify actual default values match constants
            assert_eq!(exec_config.max_concurrent_compilations, DEFAULT_MAX_CONCURRENT_COMPILATIONS);
            assert_eq!(exec_config.compilation_timeout.as_secs(), DEFAULT_COMPILATION_TIMEOUT_SECS);
            assert_eq!(exec_config.max_failed_compilations, DEFAULT_MAX_FAILED_COMPILATIONS);
            assert_eq!(exec_config.max_memory_cache_size, Some(DEFAULT_MEMORY_CACHE_SIZE));
            assert_eq!(exec_config.max_disk_cache_size, Some(DEFAULT_DISK_CACHE_SIZE_BYTES));
            assert_eq!(exec_config.memory_cache_timeout.as_millis(), DEFAULT_MEMORY_CACHE_TIMEOUT_MS as u128);
            assert_eq!(exec_config.disk_cache_load_timeout.as_secs(), DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS);
            assert_eq!(exec_config.compilation_mode, NativeCompilationMode::Async);
            assert!(exec_config.enable_retry);
            assert_eq!(exec_config.cache_dir, PathBuf::from(DEFAULT_CACHE_DIR));
        } else {
            panic!("Expected Enabled variant");
        }
    }

    #[test]
    fn test_config_builder() {
        let config = NativeConfig::builder()
            .with_max_concurrent_compilations(8)
            .with_compilation_timeout(Duration::from_secs(600))
            .with_max_failed_compilations(5000)
            .build();

        if let NativeConfig::Enabled(exec_config) = config {
            assert_eq!(exec_config.max_concurrent_compilations, 8);
            assert_eq!(exec_config.compilation_timeout.as_secs(), 600);
            assert_eq!(exec_config.max_failed_compilations, 5000);
        } else {
            panic!("Expected Enabled variant");
        }
    }

    #[test]
    fn test_config_validation() {
        use std::env::temp_dir;

        // Validation is skipped when native execution is disabled
        let disabled_config = NativeConfig::Disabled;
        assert!(disabled_config.validate().is_ok(), "Disabled config should always validate");

        // Use a temporary directory that we can actually create in tests
        let temp_cache_dir = temp_dir().join("madara_test_cache");

        // Test invalid config: max_concurrent_compilations = 0
        let invalid_config =
            NativeConfig::builder().with_max_concurrent_compilations(0).with_cache_dir(temp_cache_dir.clone()).build();
        assert!(invalid_config.validate().is_err(), "Should fail validation with max_concurrent_compilations = 0");

        // Test invalid config: compilation_timeout = 0
        let invalid_config2 = NativeConfig::builder()
            .with_max_concurrent_compilations(4)
            .with_compilation_timeout(Duration::from_secs(0))
            .with_cache_dir(temp_cache_dir.clone())
            .build();
        assert!(invalid_config2.validate().is_err(), "Should fail validation with compilation_timeout = 0");

        // Test that validation passes with valid config
        let valid_config = NativeConfig::builder()
            .with_max_concurrent_compilations(4)
            .with_compilation_timeout(Duration::from_secs(300))
            .with_cache_dir(temp_cache_dir.clone())
            .build();
        assert!(valid_config.validate().is_ok(), "Should pass validation with valid config");

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_cache_dir);
    }

    #[test]
    fn test_compilation_modes() {
        // Test Async mode
        let async_config = NativeConfig::builder().with_compilation_mode(NativeCompilationMode::Async).build();
        if let NativeConfig::Enabled(ref exec_config) = async_config {
            assert_eq!(exec_config.compilation_mode, NativeCompilationMode::Async);
            assert!(!async_config.is_blocking_mode());
        } else {
            panic!("Expected Enabled variant");
        }

        // Test Blocking mode
        let blocking_config = NativeConfig::builder().with_compilation_mode(NativeCompilationMode::Blocking).build();
        if let NativeConfig::Enabled(ref exec_config) = blocking_config {
            assert_eq!(exec_config.compilation_mode, NativeCompilationMode::Blocking);
            assert!(blocking_config.is_blocking_mode());
        } else {
            panic!("Expected Enabled variant");
        }
    }

    #[test]
    fn test_missing_cache_directory_created_automatically() {
        use tempfile::TempDir;

        // Create a temp directory and then create a subdirectory path that doesn't exist
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let non_existent_cache_dir = temp_dir.path().join("cache").join("classes");

        // Verify directory doesn't exist
        assert!(!non_existent_cache_dir.exists(), "Cache directory should not exist initially");

        // Create config with non-existent cache directory using builder
        let config = NativeConfig::builder().with_cache_dir(non_existent_cache_dir.clone()).build();

        // Validate config - this should create the directory
        let validation_result = config.validate();
        assert!(validation_result.is_ok(), "Config validation should succeed and create directory");

        // Verify directory was created
        assert!(non_existent_cache_dir.exists(), "Cache directory should be created automatically by validate()");
        assert!(non_existent_cache_dir.is_dir(), "Cache directory should be a directory");
    }

    #[test]
    fn test_cache_directory_path_is_file() {
        use std::fs::File;
        use tempfile::TempDir;

        // Create a temp directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Cache directory path points to a file (not a directory)
        // This will fail the is_dir() check
        let file_path = temp_dir.path().join("cache_file");
        File::create(&file_path).expect("Failed to create file");

        assert!(file_path.exists(), "File should exist");
        assert!(file_path.is_file(), "Path should be a file, not a directory");

        let config = NativeConfig::builder().with_cache_dir(file_path.clone()).build();

        let validation_result = config.validate();
        assert!(validation_result.is_err(), "Config validation should fail when cache directory path is a file");

        let error_msg = validation_result.unwrap_err();
        assert!(
            error_msg.contains("not a directory"),
            "Error message should mention 'not a directory', got: {}",
            error_msg
        );
    }

    #[test]
    fn test_builder_pattern() {
        let config = NativeConfig::builder()
            .with_cache_dir(PathBuf::from("/tmp/test"))
            .with_max_memory_cache_size(Some(2000))
            .with_compilation_mode(NativeCompilationMode::Blocking)
            .build();

        if let NativeConfig::Enabled(exec_config) = config {
            assert_eq!(exec_config.cache_dir, PathBuf::from("/tmp/test"));
            assert_eq!(exec_config.max_memory_cache_size, Some(2000));
            assert_eq!(exec_config.compilation_mode, NativeCompilationMode::Blocking);
        } else {
            panic!("Expected Enabled variant");
        }
    }
}
