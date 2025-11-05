/// Configuration for Cairo Native compilation and caching
use std::path::PathBuf;
use std::time::Duration;

/// Native compilation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCompilationMode {
    /// Compile in background, use Cairo VM as fallback (production)
    Async,
    /// Wait for compilation, fail if compilation fails (testing/debugging)
    Blocking,
}

#[derive(Debug, Clone)]
pub struct NativeConfig {
    /// Enable Cairo Native execution (runtime control)
    /// When false, all contracts use Cairo VM regardless of cache/compilation
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
            enable_native_execution: true, // Native enabled by default
            cache_dir: std::env::var("MADARA_NATIVE_CLASSES_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/usr/share/madara/data/classes")),
            max_memory_cache_size: 1000,                  // Keep up to 1000 classes in memory
            max_disk_cache_size: 10 * 1024 * 1024 * 1024, // 10 GB
            max_concurrent_compilations: 4,
            compilation_timeout: Duration::from_secs(300),  // 5 minutes
            compilation_mode: NativeCompilationMode::Async, // Default to async
        }
    }
}

impl NativeConfig {
    /// Create a new configuration with custom values
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

/// Initialize the global configuration (call once at startup)
pub fn init_config(config: NativeConfig) -> Result<(), String> {
    config.validate()?;
    CONFIG.set(config).map_err(|_| "Configuration already initialized".to_string())?;

    let cfg = get_config();
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

/// Get the global configuration (or default if not initialized)
pub fn get_config() -> &'static NativeConfig {
    CONFIG.get_or_init(|| {
        tracing::warn!("Cairo Native config not initialized, using defaults");
        NativeConfig::default()
    })
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
}
