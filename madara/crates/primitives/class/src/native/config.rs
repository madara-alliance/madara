//! Configuration for Cairo Native compilation and caching
//!
//! This module provides configuration management for Cairo Native execution,
//! including compilation settings, cache limits, and execution modes.
//!
//! # Overview
//!
//! The configuration is initialized once at startup via `init_config()` and cannot
//! be changed afterward. This ensures consistent behavior throughout the node's lifetime.
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

/// Maximum allowed class hash hex string length (safety check)
pub const MAX_CLASS_HASH_HEX_LENGTH: usize = 100;

/// Default disk cache size: 10 GB
pub const DEFAULT_DISK_CACHE_SIZE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Default compilation timeout: 5 minutes
pub const DEFAULT_COMPILATION_TIMEOUT_SECS: u64 = 300;

/// Default memory cache size: 1000 classes
pub const DEFAULT_MEMORY_CACHE_SIZE: usize = 1000;

/// Default maximum concurrent compilations
pub const DEFAULT_MAX_CONCURRENT_COMPILATIONS: usize = 4;

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

    /// Maximum number of classes to keep in memory cache (0 = unlimited)
    pub max_memory_cache_size: usize,

    /// Maximum disk cache size in bytes (0 = unlimited)
    pub max_disk_cache_size: u64,

    /// Maximum number of concurrent compilations
    pub max_concurrent_compilations: usize,

    /// Maximum time to wait for a single compilation
    pub compilation_timeout: Duration,

    /// Compilation mode (async or blocking)
    pub compilation_mode: NativeCompilationMode,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            enable_native_execution: false, // Native disabled by default
            cache_dir: std::env::var("MADARA_NATIVE_CLASSES_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/usr/share/madara/data/classes")),
            max_memory_cache_size: DEFAULT_MEMORY_CACHE_SIZE,
            max_disk_cache_size: DEFAULT_DISK_CACHE_SIZE_BYTES,
            max_concurrent_compilations: DEFAULT_MAX_CONCURRENT_COMPILATIONS,
            compilation_timeout: Duration::from_secs(DEFAULT_COMPILATION_TIMEOUT_SECS),
            compilation_mode: NativeCompilationMode::Async, // Default to async
        }
    }
}

impl NativeConfig {
    /// Create a new configuration with default values.
    ///
    /// Default values:
    /// - `enable_native_execution`: `false` (disabled)
    /// - `cache_dir`: `/usr/share/madara/data/classes` (or `MADARA_NATIVE_CLASSES_PATH` env var)
    /// - `max_memory_cache_size`: 1000 classes
    /// - `max_disk_cache_size`: 10 GB
    /// - `max_concurrent_compilations`: 4
    /// - `compilation_timeout`: 5 minutes
    /// - `compilation_mode`: `Async`
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

    /// Set the maximum memory cache size
    pub fn with_max_memory_cache_size(mut self, size: usize) -> Self {
        self.max_memory_cache_size = size;
        self
    }

    /// Set the maximum disk cache size in bytes
    pub fn with_max_disk_cache_size(mut self, size: u64) -> Self {
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

    /// Set the compilation mode
    pub fn with_compilation_mode(mut self, mode: NativeCompilationMode) -> Self {
        self.compilation_mode = mode;
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

/// Global configuration instance
static CONFIG: std::sync::OnceLock<NativeConfig> = std::sync::OnceLock::new();

/// Initialize the global configuration (call once at startup).
///
/// This function validates the configuration and sets it globally. It can only
/// be called once - subsequent calls will return an error.
///
/// The configuration is typically initialized from CLI parameters via
/// `CairoNativeParams::to_runtime_config()`.
///
/// Also initializes the compilation semaphore with the configured concurrency limit.
///
/// # Errors
///
/// Returns an error if:
/// - Configuration validation fails (invalid paths, zero limits, etc.)
/// - Configuration was already initialized
pub fn init_config(config: NativeConfig) -> Result<(), String> {
    config.validate()?;
    CONFIG.set(config).map_err(|_| "Configuration already initialized".to_string())?;

    let cfg = get_config();

    // Initialize the compilation semaphore with the actual config value
    // This must be done after config is set
    crate::native::init_compilation_semaphore(cfg.max_concurrent_compilations);
    if cfg.enable_native_execution {
        let mode_description = match cfg.compilation_mode {
            NativeCompilationMode::Async => "Async (VM fallback enabled)",
            NativeCompilationMode::Blocking => "Blocking (strictly native, no VM fallback)",
        };
        tracing::info!(
            "🚀 Cairo Native ENABLED: cache_dir={:?}, mode={}, max_memory_cache={}, max_concurrent={}",
            cfg.cache_dir,
            mode_description,
            cfg.max_memory_cache_size,
            cfg.max_concurrent_compilations
        );
    } else {
        tracing::info!("🐪 Cairo Native DISABLED: All contracts will use Cairo VM execution");
    }

    Ok(())
}

/// Get the global configuration (or default if not initialized).
///
/// If the configuration hasn't been initialized yet, returns the default configuration
/// (with native execution disabled). A warning is logged in this case.
///
/// This function is safe to call from any thread.
pub fn get_config() -> &'static NativeConfig {
    CONFIG.get_or_init(|| {
        tracing::warn!("Cairo Native config not initialized, using defaults");
        NativeConfig::default()
    })
}

/// Get the current max concurrent compilations from config.
///
/// This function reads the config each time to avoid stale semaphore values.
/// Used by the compilation semaphore to respect dynamic config changes.
pub fn get_max_concurrent_compilations() -> usize {
    get_config().max_concurrent_compilations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NativeConfig::default();
        assert!(config.max_concurrent_compilations > 0);
        assert!(config.compilation_timeout.as_secs() > 0);
    }

    #[test]
    fn test_config_builder() {
        let config =
            NativeConfig::new().with_max_concurrent_compilations(8).with_compilation_timeout(Duration::from_secs(600));

        assert_eq!(config.max_concurrent_compilations, 8);
        assert_eq!(config.compilation_timeout.as_secs(), 600);
    }

    #[test]
    fn test_config_validation() {
        let mut config = NativeConfig { max_concurrent_compilations: 0, ..Default::default() };
        assert!(config.validate().is_err());

        config.max_concurrent_compilations = 4;
        config.compilation_timeout = Duration::from_secs(0);
        assert!(config.validate().is_err());
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
}
