use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Cairo Native execution parameters
#[derive(Clone, Debug, clap::Args, Deserialize, Serialize)]
pub struct CairoNativeParams {
    /// Enable Cairo Native execution.
    /// When disabled, all contracts will use Cairo VM execution regardless of cache.
    #[clap(
        env = "MADARA_ENABLE_NATIVE_EXECUTION",
        long,
        default_value = "true",
        action = clap::ArgAction::Set,
        value_name = "BOOL"
    )]
    pub enable_native_execution: bool,

    /// Directory path for storing compiled native classes (.so files).
    /// These files can be reused across node restarts.
    /// Default: /usr/share/madara/data/classes
    #[clap(env = "MADARA_NATIVE_CACHE_DIR", long, value_name = "PATH")]
    pub native_cache_dir: Option<PathBuf>,

    /// Maximum number of native classes to keep in memory cache.
    /// Set to 0 for unlimited (not recommended for production).
    /// Higher values improve performance but use more RAM.
    #[clap(env = "MADARA_NATIVE_MAX_MEMORY_CACHE", long, default_value = "1000", value_name = "SIZE")]
    pub native_max_memory_cache_size: usize,

    /// Maximum disk cache size in bytes for compiled native classes.
    /// Set to 0 for unlimited. Old files will be cleaned up when limit is reached.
    #[clap(env = "MADARA_NATIVE_MAX_DISK_CACHE_BYTES", long, value_name = "BYTES")]
    pub native_max_disk_cache_size: Option<u64>,

    /// Maximum number of concurrent native compilations.
    /// Lower values reduce CPU usage during sync, higher values speed up initial compilation.
    #[clap(env = "MADARA_NATIVE_MAX_CONCURRENT_COMPILATIONS", long, default_value = "4", value_name = "COUNT")]
    pub native_max_concurrent_compilations: usize,

    /// Maximum time to wait for a single native compilation (in seconds).
    /// Compilation will be aborted after this timeout.
    #[clap(env = "MADARA_NATIVE_COMPILATION_TIMEOUT_SECS", long, default_value = "300", value_name = "SECONDS")]
    pub native_compilation_timeout_secs: u64,

    /// Native compilation mode: 'async' (default) or 'blocking'.
    /// - async: Compile in background, use Cairo VM as fallback (production)
    /// - blocking: Wait for compilation, fail if it fails (testing/debugging)
    #[clap(env = "MADARA_NATIVE_COMPILATION_MODE", long, default_value = "async", value_name = "MODE")]
    pub native_compilation_mode: String,
}

impl Default for CairoNativeParams {
    fn default() -> Self {
        Self {
            enable_native_execution: true, // Native enabled by default
            native_cache_dir: None,
            native_max_memory_cache_size: 1000,
            native_max_disk_cache_size: Some(10 * 1024 * 1024 * 1024), // 10 GB
            native_max_concurrent_compilations: 4,
            native_compilation_timeout_secs: 300, // 5 minutes
            native_compilation_mode: "async".to_string(),
        }
    }
}

impl CairoNativeParams {
    /// Get the cache directory, using default if not specified
    pub fn cache_dir(&self) -> PathBuf {
        self.native_cache_dir
            .clone()
            .or_else(|| std::env::var("MADARA_NATIVE_CLASSES_PATH").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/usr/share/madara/data/classes"))
    }

    /// Get the compilation timeout as a Duration
    pub fn compilation_timeout(&self) -> Duration {
        Duration::from_secs(self.native_compilation_timeout_secs)
    }

    /// Get the max disk cache size in bytes
    pub fn max_disk_cache_size(&self) -> u64 {
        self.native_max_disk_cache_size.unwrap_or(10 * 1024 * 1024 * 1024)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Skip all validation if native execution is disabled
        if !self.enable_native_execution {
            return Ok(());
        }

        // Native execution is enabled - validate all parameters
        if self.native_max_concurrent_compilations == 0 {
            return Err("native_max_concurrent_compilations must be greater than 0".to_string());
        }

        if self.native_compilation_timeout_secs == 0 {
            return Err("native_compilation_timeout_secs must be greater than 0".to_string());
        }

        // Try to create cache directory if it doesn't exist
        let cache_dir = self.cache_dir();
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to create cache directory {:?}: {}", cache_dir, e))?;
        }

        // Check if directory is writable
        if !cache_dir.is_dir() {
            return Err(format!("Cache path {:?} is not a directory", cache_dir));
        }

        Ok(())
    }

    /// Convert CLI params to runtime config for mp-class
    pub fn to_runtime_config(&self) -> mp_class::native_config::NativeConfig {
        use mp_class::native_config::NativeCompilationMode;

        // Parse compilation mode from string
        let mode = match self.native_compilation_mode.to_lowercase().as_str() {
            "blocking" => NativeCompilationMode::Blocking,
            _ => NativeCompilationMode::Async, // Default to async for any other value
        };

        mp_class::native_config::NativeConfig::new()
            .with_native_execution(self.enable_native_execution)
            .with_cache_dir(self.cache_dir())
            .with_max_memory_cache_size(self.native_max_memory_cache_size)
            .with_max_disk_cache_size(self.max_disk_cache_size())
            .with_max_concurrent_compilations(self.native_max_concurrent_compilations)
            .with_compilation_timeout(self.compilation_timeout())
            .with_compilation_mode(mode)
    }
}
