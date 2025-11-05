//! Cairo Native execution CLI parameters
//!
//! This module provides command-line arguments and environment variable support for
//! configuring Cairo Native execution. Cairo Native compiles Cairo contracts to native
//! machine code for faster execution compared to the Cairo VM.
//!
//! # Quick Start
//!
//! To enable Cairo Native execution, you must set `--enable-native-execution true`.
//! All other parameters have sensible defaults and are optional.
//!
//! ```bash
//! # Enable native execution with default settings
//! madara --enable-native-execution true
//!
//! # Customize cache limits
//! madara --enable-native-execution true \
//!        --native-max-memory-cache 2000 \
//!        --native-max-disk-cache-bytes 21474836480  # 20 GB
//!
//! # Use unlimited cache
//! madara --enable-native-execution true \
//!        --native-max-memory-cache unlimited
//! ```
//!
//! # Configuration Methods
//!
//! Parameters can be set via:
//! - Command-line arguments: `--native-max-memory-cache 2000`
//! - Environment variables: `MADARA_NATIVE_MAX_MEMORY_CACHE=2000`
//! - Configuration files: Serialize `CairoNativeParams` to JSON/TOML
//!
//! # Integration
//!
//! To include these parameters in your CLI application:
//!
//! ```rust
//! #[derive(clap::Parser)]
//! struct Cli {
//!     #[clap(flatten)]
//!     pub cairo_native_params: CairoNativeParams,
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

// Import default constants from the native config module
use mp_class::native_config::{
    DEFAULT_COMPILATION_TIMEOUT_SECS, DEFAULT_DISK_CACHE_SIZE_BYTES, DEFAULT_MAX_CONCURRENT_COMPILATIONS,
    DEFAULT_MEMORY_CACHE_SIZE,
};

/// Validates and normalizes compilation mode input.
///
/// Accepts "async" or "blocking" (case-insensitive) and returns the lowercase version.
/// Returns an error for any other value.
fn parse_compilation_mode(s: &str) -> Result<String, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "async" | "blocking" => Ok(lower),
        _ => Err(format!("Invalid compilation mode: {}. Must be 'async' or 'blocking'", s)),
    }
}

/// Parses a memory cache size value from user input.
///
/// Accepts:
/// - A positive integer: the maximum number of classes to cache
/// - "unlimited": removes the cache limit (no eviction)
///
/// Returns an error if the value is invalid or zero (zero is not allowed as a limit).
fn parse_cache_size(s: &str) -> Result<usize, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "unlimited" => Ok(usize::MAX), // Use sentinel value for unlimited
        _ => {
            let value = s.parse::<usize>().map_err(|e| format!("Invalid cache size '{}': {}", s, e))?;
            if value == 0 {
                return Err("Cache size must be greater than 0. Use 'unlimited' for unlimited cache.".to_string());
            }
            if value == usize::MAX {
                return Err(format!(
                    "Cache size cannot be {} (reserved for unlimited). Use 'unlimited' instead.",
                    usize::MAX
                ));
            }
            Ok(value)
        }
    }
}

/// Parses a disk cache size value from user input.
///
/// Accepts:
/// - A positive integer (in bytes): the maximum disk space for cached classes
/// - "unlimited": removes the disk cache limit (no cleanup)
///
/// Returns an error if the value is invalid or zero (zero is not allowed as a limit).
fn parse_disk_cache_size(s: &str) -> Result<u64, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "unlimited" => Ok(u64::MAX), // Use sentinel value for unlimited
        _ => {
            let value = s.parse::<u64>().map_err(|e| format!("Invalid disk cache size '{}': {}", s, e))?;
            if value == 0 {
                return Err("Disk cache size must be greater than 0. Use 'unlimited' for unlimited cache.".to_string());
            }
            if value == u64::MAX {
                return Err(format!(
                    "Disk cache size cannot be {} (reserved for unlimited). Use 'unlimited' instead.",
                    u64::MAX
                ));
            }
            Ok(value)
        }
    }
}

/// Configuration parameters for Cairo Native execution.
///
/// These parameters control how contracts are compiled and executed using Cairo Native,
/// which provides faster execution than the Cairo VM by compiling contracts to native
/// machine code.
///
/// # When to Use
///
/// - **Production**: Enable for better performance, use `async` mode for reliability
/// - **Development**: Can use `blocking` mode for immediate feedback on compilation errors
/// - **Memory-constrained**: Reduce cache sizes or use unlimited to trade memory for speed
///
/// # Performance Considerations
///
/// - Native execution is significantly faster than VM execution
/// - Compiled classes are cached on disk and reused across restarts
/// - Memory cache provides instant access to frequently-used classes
/// - Larger caches improve performance but use more RAM and disk space
#[derive(Clone, Debug, clap::Args, Deserialize, Serialize)]
pub struct CairoNativeParams {
    /// Enable or disable Cairo Native execution.
    ///
    /// When `false` (default), all contracts execute using the Cairo VM regardless of
    /// whether native compiled versions are available. This ensures backward compatibility.
    ///
    /// When `true`, native-compiled contracts are used when available, falling back to
    /// VM execution only if compilation hasn't completed yet (in async mode).
    #[clap(
        env = "MADARA_ENABLE_NATIVE_EXECUTION",
        long,
        default_value = "false",
        action = clap::ArgAction::Set,
        value_name = "BOOL"
    )]
    pub enable_native_execution: bool,

    /// Directory where compiled native classes are stored.
    ///
    /// Compiled classes are saved as `.so` files (shared libraries) and persist across
    /// node restarts, avoiding recompilation. The directory is created automatically if
    /// it doesn't exist.
    ///
    /// Default: `/usr/share/madara/data/classes`
    ///
    /// Can also be set via the `MADARA_NATIVE_CLASSES_PATH` environment variable.
    #[clap(env = "MADARA_NATIVE_CACHE_DIR", long, value_name = "PATH")]
    pub native_cache_dir: Option<PathBuf>,

    /// Maximum number of compiled classes to keep in memory.
    ///
    /// Frequently-used classes are kept in memory for instant access. When the limit is
    /// reached, the least recently used classes are evicted to make room for new ones.
    ///
    /// - **Default**: 1000 classes
    /// - **Unlimited**: Set to `"unlimited"` to disable eviction (keeps all classes in memory)
    /// - **Custom**: Provide any positive integer (e.g., `2000`)
    ///
    /// Higher values improve performance for frequently-used contracts but consume more RAM.
    /// A class uses approximately 1-10 MB in memory depending on its size.
    #[clap(
        env = "MADARA_NATIVE_MAX_MEMORY_CACHE",
        long,
        default_value_t = DEFAULT_MEMORY_CACHE_SIZE,
        value_parser = parse_cache_size,
        value_name = "SIZE"
    )]
    pub native_max_memory_cache_size: usize,

    /// Maximum disk space (in bytes) for storing compiled classes.
    ///
    /// Compiled classes are saved to disk and reused across restarts. When the limit is
    /// reached, the oldest files are automatically deleted to make room for new compilations.
    ///
    /// - **Default**: 10 GB (10,737,418,240 bytes)
    /// - **Unlimited**: Set to `"unlimited"` to disable disk space limits (no cleanup)
    /// - **Custom**: Provide size in bytes (e.g., `21474836480` for 20 GB)
    ///
    /// Use larger values if you work with many contracts or want to preserve compilations
    /// across long periods. A typical compiled class is 100 KB - 10 MB.
    #[clap(
        env = "MADARA_NATIVE_MAX_DISK_CACHE_BYTES",
        long,
        default_value_t = DEFAULT_DISK_CACHE_SIZE_BYTES,
        value_parser = parse_disk_cache_size,
        value_name = "BYTES"
    )]
    pub native_max_disk_cache_size: u64,

    /// Maximum number of contracts that can be compiled simultaneously.
    ///
    /// During initial sync or when encountering new contracts, multiple compilations can
    /// run in parallel. This setting controls how many run at once.
    ///
    /// - **Default**: 4 concurrent compilations
    /// - **Lower values** (1-2): Reduce CPU usage, slower initial compilation
    /// - **Higher values** (8-16): Faster compilation, higher CPU usage
    ///
    /// Increase this if you have many CPU cores and want faster initial sync.
    /// Decrease if you want to reserve CPU for other processes.
    #[clap(
        env = "MADARA_NATIVE_MAX_CONCURRENT_COMPILATIONS",
        long,
        default_value_t = DEFAULT_MAX_CONCURRENT_COMPILATIONS,
        value_name = "COUNT"
    )]
    pub native_max_concurrent_compilations: usize,

    /// Maximum time to wait for a single contract compilation to complete.
    ///
    /// If a compilation takes longer than this timeout, it's aborted and the contract
    /// falls back to VM execution (in async mode) or the transaction fails (in blocking mode).
    ///
    /// - **Default**: 300 seconds (5 minutes)
    /// - **Recommended**: 60-600 seconds depending on contract complexity
    ///
    /// Increase this timeout for very large or complex contracts that take longer to compile.
    /// Most contracts compile in under 30 seconds.
    #[clap(
        env = "MADARA_NATIVE_COMPILATION_TIMEOUT_SECS",
        long,
        default_value_t = DEFAULT_COMPILATION_TIMEOUT_SECS,
        value_name = "SECONDS"
    )]
    pub native_compilation_timeout_secs: u64,

    /// How compilation failures are handled.
    ///
    /// - **`async`** (default): Compilations run in the background. Contracts use VM execution
    ///   immediately while compilation happens. Best for production where availability is critical.
    ///
    /// - **`blocking`**: Waits for compilation to complete before executing. If compilation fails,
    ///   the transaction fails. Useful for testing or when you need strict native execution.
    #[clap(
        env = "MADARA_NATIVE_COMPILATION_MODE",
        long,
        default_value = "async",
        value_name = "MODE",
        value_parser = parse_compilation_mode,
        ignore_case = true
    )]
    pub native_compilation_mode: String,
}

impl Default for CairoNativeParams {
    fn default() -> Self {
        Self {
            enable_native_execution: false, // Native disabled by default
            native_cache_dir: None,
            native_max_memory_cache_size: DEFAULT_MEMORY_CACHE_SIZE,
            native_max_disk_cache_size: DEFAULT_DISK_CACHE_SIZE_BYTES,
            native_max_concurrent_compilations: DEFAULT_MAX_CONCURRENT_COMPILATIONS,
            native_compilation_timeout_secs: DEFAULT_COMPILATION_TIMEOUT_SECS,
            native_compilation_mode: "async".to_string(),
        }
    }
}

impl CairoNativeParams {
    /// Returns the directory path where compiled classes are stored.
    ///
    /// The path is determined by checking (in order):
    /// 1. The `native_cache_dir` CLI parameter if set
    /// 2. The `MADARA_NATIVE_CLASSES_PATH` environment variable if set
    /// 3. The default path: `/usr/share/madara/data/classes`
    pub fn cache_dir(&self) -> PathBuf {
        self.native_cache_dir
            .clone()
            .or_else(|| std::env::var("MADARA_NATIVE_CLASSES_PATH").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/usr/share/madara/data/classes"))
    }

    /// Returns the compilation timeout as a `Duration` for use with async timers.
    pub fn compilation_timeout(&self) -> Duration {
        Duration::from_secs(self.native_compilation_timeout_secs)
    }

    /// Validates all configuration parameters and ensures the cache directory exists.
    ///
    /// Returns an error if:
    /// - Any numeric parameter is set to zero
    /// - The cache directory cannot be created or is not writable
    /// - The cache path exists but is not a directory
    ///
    /// This should be called before `to_runtime_config()` to catch configuration errors early.
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

        // Validate cache limits (must be > 0, or usize::MAX/u64::MAX for unlimited)
        if self.native_max_memory_cache_size == 0 {
            return Err(
                "native_max_memory_cache_size must be greater than 0. Use 'unlimited' for unlimited cache.".to_string()
            );
        }
        if self.native_max_disk_cache_size == 0 {
            return Err(
                "native_max_disk_cache_size must be greater than 0. Use 'unlimited' for unlimited cache.".to_string()
            );
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

    /// Converts CLI parameters into the runtime configuration used by the compilation system.
    ///
    /// This performs the final transformation from user-facing CLI parameters to the internal
    /// configuration format. Call this once at startup after calling `validate()`.
    ///
    /// # Transformations
    ///
    /// - Compilation mode strings are normalized and converted to enum variants
    /// - Cache size values of "unlimited" are converted to `None` (no limit)
    /// - Default values are already applied, so this just handles format conversion
    ///
    /// # Panics
    ///
    /// This function does not validate inputs. Always call `validate()` first.
    pub fn to_runtime_config(&self) -> mp_class::native_config::NativeConfig {
        use mp_class::native_config::NativeCompilationMode;

        // Normalize compilation mode: parse string and convert to enum
        let mode = match self.native_compilation_mode.to_lowercase().as_str() {
            "blocking" => NativeCompilationMode::Blocking,
            _ => NativeCompilationMode::Async, // Default to async for any unrecognized value
        };

        // Convert cache sizes: "unlimited" becomes None, numeric values become Some(n)
        let max_memory_cache_size = if self.native_max_memory_cache_size == usize::MAX {
            None // User requested unlimited cache
        } else {
            Some(self.native_max_memory_cache_size) // User specified a numeric limit
        };

        let max_disk_cache_size = if self.native_max_disk_cache_size == u64::MAX {
            None // User requested unlimited disk cache
        } else {
            Some(self.native_max_disk_cache_size) // User specified a numeric limit in bytes
        };

        mp_class::native_config::NativeConfig::new()
            .with_native_execution(self.enable_native_execution)
            .with_cache_dir(self.cache_dir())
            .with_max_memory_cache_size(max_memory_cache_size)
            .with_max_disk_cache_size(max_disk_cache_size)
            .with_max_concurrent_compilations(self.native_max_concurrent_compilations)
            .with_compilation_timeout(self.compilation_timeout())
            .with_compilation_mode(mode)
    }
}
