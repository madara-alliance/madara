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

/// Configuration for Cairo Native execution and compilation.
///
/// This struct holds all runtime configuration for native execution.
/// It is initialized once at startup and cannot be changed afterward.
#[derive(Debug, Clone)]
pub struct NativeConfig {
    /// Enable Cairo Native execution (runtime control).
    ///
    /// When `false`, all contracts use Cairo VM regardless of cache state or
    /// compilation availability. This is the default and ensures backward compatibility.
    ///
    /// Set via CLI flag `--enable-native-execution` or environment variable
    /// `MADARA_ENABLE_NATIVE_EXECUTION`.
    pub enable_native_execution: bool,

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

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            enable_native_execution: false,              // Native disabled by default
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

impl NativeConfig {
    /// Create a new configuration with default values.
    ///
    /// Default values:
    /// - `enable_native_execution`: `false` (disabled)
    /// - `cache_dir`: `/usr/share/madara/data/classes` (should be set via CLI/config, not env var)
    /// - `max_memory_cache_size`: Some(1000) classes (None = unlimited)
    /// - `max_disk_cache_size`: Some(10 GB) (None = unlimited)
    /// - `max_concurrent_compilations`: 4
    /// - `compilation_timeout`: 5 minutes
    /// - `compilation_mode`: `Async`
    ///
    /// Note: The cache directory should be set via `with_cache_dir()` from CLI parameters,
    /// not from environment variables. The CLI layer handles environment variable parsing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable native execution (runtime control)
    pub fn with_native_execution(mut self, enabled: bool) -> Self {
        self.enable_native_execution = enabled;
        self
    }

    /// Set the cache directory
    pub fn with_cache_dir(mut self, path: PathBuf) -> Self {
        self.cache_dir = path;
        self
    }

    /// Set the maximum memory cache size (None = unlimited)
    pub fn with_max_memory_cache_size(mut self, size: Option<usize>) -> Self {
        self.max_memory_cache_size = size;
        self
    }

    /// Set the maximum disk cache size in bytes (None = unlimited)
    pub fn with_max_disk_cache_size(mut self, size: Option<u64>) -> Self {
        self.max_disk_cache_size = size;
        self
    }

    /// Set the maximum number of concurrent compilations
    pub fn with_max_concurrent_compilations(mut self, max: usize) -> Self {
        self.max_concurrent_compilations = max;
        self
    }

    /// Set the compilation timeout
    pub fn with_compilation_timeout(mut self, timeout: Duration) -> Self {
        self.compilation_timeout = timeout;
        self
    }

    /// Set the memory cache lookup timeout
    pub fn with_memory_cache_timeout(mut self, timeout: Duration) -> Self {
        self.memory_cache_timeout = timeout;
        self
    }

    /// Set the disk cache load timeout
    pub fn with_disk_cache_load_timeout(mut self, timeout: Duration) -> Self {
        self.disk_cache_load_timeout = timeout;
        self
    }

    /// Set the compilation mode
    pub fn with_compilation_mode(mut self, mode: NativeCompilationMode) -> Self {
        self.compilation_mode = mode;
        self
    }

    /// Enable or disable automatic retry of failed compilations
    pub fn with_enable_retry(mut self, enabled: bool) -> Self {
        self.enable_retry = enabled;
        self
    }

    /// Set the maximum number of failed compilation entries to keep
    pub fn with_max_failed_compilations(mut self, max: usize) -> Self {
        self.max_failed_compilations = max;
        self
    }

    /// Check if blocking mode is enabled
    pub fn is_blocking_mode(&self) -> bool {
        matches!(self.compilation_mode, NativeCompilationMode::Blocking)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Skip all validation if native execution is disabled
        if !self.enable_native_execution {
            return Ok(());
        }

        // Native execution is enabled - validate all parameters
        if self.max_concurrent_compilations == 0 {
            return Err("max_concurrent_compilations must be greater than 0".to_string());
        }

        // Validate cache limits if set (must be > 0 if Some)
        if let Some(size) = self.max_memory_cache_size {
            if size == 0 {
                return Err("max_memory_cache_size must be greater than 0 if set".to_string());
            }
        }
        if let Some(size) = self.max_disk_cache_size {
            if size == 0 {
                return Err("max_disk_cache_size must be greater than 0 if set".to_string());
            }
        }

        if self.compilation_timeout.as_secs() == 0 {
            return Err("compilation_timeout must be greater than 0".to_string());
        }

        // Try to create cache directory if it doesn't exist
        if !self.cache_dir.exists() {
            std::fs::create_dir_all(&self.cache_dir)
                .map_err(|e| format!("Failed to create cache directory {:?}: {}", self.cache_dir, e))?;
        }

        // Check if directory is writable
        if !self.cache_dir.is_dir() {
            return Err(format!("Cache path {:?} is not a directory", self.cache_dir));
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

    // Initialize the compilation semaphore with the configured concurrency limit
    crate::compilation::init_compilation_semaphore(config.max_concurrent_compilations);

    // Register OTEL metrics (will work even if OTEL isn't initialized yet, but won't export)
    // Metrics are registered lazily on first access, but we register here to ensure
    // they're available as soon as OTEL is initialized
    let _ = crate::metrics::metrics(); // Force lazy initialization

    // Log cache directory initialization (always logged, regardless of enable_native_execution)
    tracing::info!("💾 Cairo-native directory initialized: {}", config.cache_dir.display());

    // Log configuration
    if config.enable_native_execution {
        let mode_description = match config.compilation_mode {
            NativeCompilationMode::Async => "Async (VM fallback enabled)",
            NativeCompilationMode::Blocking => "Blocking (strictly native, no VM fallback)",
        };
        let memory_cache_desc = match config.max_memory_cache_size {
            Some(size) => format!("{} classes", size),
            None => "unlimited".to_string(),
        };
        let disk_cache_desc = match config.max_disk_cache_size {
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
            config.max_concurrent_compilations
        );
    } else {
        tracing::info!("Cairo native disabled - all contracts will use Cairo VM");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NativeConfig::default();
        assert!(config.max_concurrent_compilations > 0);
        assert!(config.compilation_timeout.as_secs() > 0);
        assert!(config.max_failed_compilations > 0);
    }

    #[test]
    fn test_config_builder() {
        let config = NativeConfig::new()
            .with_max_concurrent_compilations(8)
            .with_compilation_timeout(Duration::from_secs(600))
            .with_max_failed_compilations(5000);

        assert_eq!(config.max_concurrent_compilations, 8);
        assert_eq!(config.compilation_timeout.as_secs(), 600);
        assert_eq!(config.max_failed_compilations, 5000);
    }

    #[test]
    fn test_config_validation() {
        use std::env::temp_dir;

        // Validation is skipped when native execution is disabled, so enable it first
        // Use a temporary directory that we can actually create in tests
        let temp_cache_dir = temp_dir().join("madara_test_cache");

        let mut config = NativeConfig {
            enable_native_execution: true,
            max_concurrent_compilations: 0,
            cache_dir: temp_cache_dir.clone(),
            ..Default::default()
        };
        assert!(config.validate().is_err(), "Should fail validation with max_concurrent_compilations = 0");

        config.max_concurrent_compilations = 4;
        config.compilation_timeout = Duration::from_secs(0);
        config.cache_dir = temp_cache_dir.clone(); // Ensure cache_dir is still set
        assert!(config.validate().is_err(), "Should fail validation with compilation_timeout = 0");

        // Test that validation passes with valid config
        config.compilation_timeout = Duration::from_secs(300);
        config.cache_dir = temp_cache_dir.clone(); // Ensure cache_dir is still set
        assert!(config.validate().is_ok(), "Should pass validation with valid config");

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_cache_dir);
    }

    #[test]
    fn test_compilation_modes() {
        // Test Async mode
        let async_config = NativeConfig::new().with_compilation_mode(NativeCompilationMode::Async);
        assert_eq!(async_config.compilation_mode, NativeCompilationMode::Async);
        assert!(!async_config.is_blocking_mode());

        // Test Blocking mode
        let blocking_config = NativeConfig::new().with_compilation_mode(NativeCompilationMode::Blocking);
        assert_eq!(blocking_config.compilation_mode, NativeCompilationMode::Blocking);
        assert!(blocking_config.is_blocking_mode());
    }

    #[test]
    fn test_missing_cache_directory_created_automatically() {
        use tempfile::TempDir;

        // Create a temp directory and then create a subdirectory path that doesn't exist
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let non_existent_cache_dir = temp_dir.path().join("cache").join("classes");

        // Verify directory doesn't exist
        assert!(!non_existent_cache_dir.exists(), "Cache directory should not exist initially");

        // Create config with non-existent cache directory
        let config = NativeConfig::default().with_native_execution(true).with_cache_dir(non_existent_cache_dir.clone());

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

        let config = NativeConfig::default().with_native_execution(true).with_cache_dir(file_path.clone());

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
    fn test_cache_directory_creation_failure() {
        use std::fs::File;
        use tempfile::TempDir;

        // Create a temp directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // Cache directory path has a parent that is a file
        // This will cause create_dir_all to fail
        let parent_file = temp_dir.path().join("parent_file");
        File::create(&parent_file).expect("Failed to create parent file");
        let invalid_cache_dir = parent_file.join("cache");

        // Verify parent is a file
        assert!(parent_file.is_file(), "Parent should be a file");
        assert!(!invalid_cache_dir.exists(), "Cache directory should not exist");

        let config = NativeConfig::default().with_native_execution(true).with_cache_dir(invalid_cache_dir.clone());

        // Validate config - this should fail because create_dir_all cannot create a directory
        // when the parent path is a file
        let validation_result = config.validate();
        assert!(validation_result.is_err(), "Config validation should fail when parent path is a file");

        let error_msg = validation_result.unwrap_err();
        assert!(
            error_msg.contains("Failed to create cache directory"),
            "Error message should mention 'Failed to create cache directory', got: {}",
            error_msg
        );
    }
}
