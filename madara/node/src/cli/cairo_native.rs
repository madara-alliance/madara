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
use mc_cairo_native::config::{
    DEFAULT_CACHE_DIR, DEFAULT_COMPILATION_TIMEOUT_SECS, DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS,
    DEFAULT_DISK_CACHE_SIZE_BYTES, DEFAULT_MAX_CONCURRENT_COMPILATIONS, DEFAULT_MAX_FAILED_COMPILATIONS,
    DEFAULT_MEMORY_CACHE_SIZE, DEFAULT_MEMORY_CACHE_TIMEOUT_MS,
};

/// Default function for serde deserialization of memory cache size
fn default_memory_cache_size() -> usize {
    DEFAULT_MEMORY_CACHE_SIZE
}

/// Default function for serde deserialization of disk cache size
fn default_disk_cache_size() -> u64 {
    DEFAULT_DISK_CACHE_SIZE_BYTES
}

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
    /// Can also be set via the `MADARA_NATIVE_CACHE_DIR` environment variable.
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
        env = "MADARA_NATIVE_MAX_MEMORY_CACHE_CLASSES",
        long = "native-max-memory-cache-classes",
        default_value_t = DEFAULT_MEMORY_CACHE_SIZE,
        value_parser = parse_cache_size,
        value_name = "CLASSES"
    )]
    #[serde(default = "default_memory_cache_size")]
    pub native_max_memory_cache_classes: usize,

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
        long = "native-max-disk-cache-bytes",
        default_value_t = DEFAULT_DISK_CACHE_SIZE_BYTES,
        value_parser = parse_disk_cache_size,
        value_name = "BYTES"
    )]
    #[serde(default = "default_disk_cache_size")]
    pub native_max_disk_cache_bytes: u64,

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

    /// Maximum time to wait for memory cache lookup to complete.
    ///
    /// If a memory cache lookup takes longer than this timeout, it's treated as a cache miss
    /// and the system falls back to disk cache or compilation.
    ///
    /// - **Default**: 100 milliseconds
    /// - **Recommended**: 50-500 milliseconds depending on system performance
    ///
    /// Increase this timeout if you experience frequent memory cache timeouts on slower systems.
    /// Most lookups complete in under 10 milliseconds.
    #[clap(
        env = "MADARA_NATIVE_MEMORY_CACHE_TIMEOUT_MS",
        long,
        default_value_t = DEFAULT_MEMORY_CACHE_TIMEOUT_MS,
        value_name = "MILLISECONDS"
    )]
    pub native_memory_cache_timeout_ms: u64,

    /// Maximum time to wait for disk cache load to complete.
    ///
    /// If loading a compiled class from disk takes longer than this timeout, it's treated as
    /// a cache miss and the system falls back to compilation.
    ///
    /// - **Default**: 2 seconds
    /// - **Recommended**: 1-5 seconds depending on disk I/O performance
    ///
    /// Increase this timeout if you experience frequent disk cache timeouts on slower storage.
    /// Most disk loads complete in under 500 milliseconds.
    #[clap(
        env = "MADARA_NATIVE_DISK_CACHE_LOAD_TIMEOUT_SECS",
        long,
        default_value_t = DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS,
        value_name = "SECONDS"
    )]
    pub native_disk_cache_load_timeout_secs: u64,

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

    /// Enable automatic retry of failed compilations.
    ///
    /// When `true`, classes that failed compilation in async mode are automatically retried
    /// on the next request. This helps recover from transient failures (timeouts, temporary
    /// resource constraints).
    ///
    /// When `false`, failed compilations are not retried automatically. The class will continue
    /// to use VM execution until manually recompiled or the node is restarted.
    ///
    /// Default: `true` (retry enabled)
    #[clap(
        env = "MADARA_NATIVE_ENABLE_RETRY",
        long,
        default_value = "true",
        action = clap::ArgAction::Set,
        value_name = "BOOL"
    )]
    pub native_enable_retry: bool,

    /// Maximum number of failed compilation entries to keep in memory.
    ///
    /// When compilations fail, their class hashes are stored for retry tracking. This setting
    /// limits how many failed entries are kept. When the limit is reached, the oldest entries
    /// are automatically evicted using LRU (Least Recently Used) policy.
    ///
    /// - **Default**: 10,000 entries
    /// - **Higher values**: Track more failed classes for retry, but use more memory
    /// - **Lower values**: Use less memory, but fewer failed classes can be retried
    ///
    /// Each entry uses minimal memory (~32 bytes), so even 10,000 entries is only ~320 KB.
    #[clap(
        env = "MADARA_NATIVE_MAX_FAILED_COMPILATIONS",
        long,
        default_value_t = DEFAULT_MAX_FAILED_COMPILATIONS,
        value_name = "COUNT"
    )]
    pub native_max_failed_compilations: usize,
}

impl Default for CairoNativeParams {
    fn default() -> Self {
        Self {
            enable_native_execution: false, // Native disabled by default
            native_cache_dir: None,
            native_max_memory_cache_classes: DEFAULT_MEMORY_CACHE_SIZE,
            native_max_disk_cache_bytes: DEFAULT_DISK_CACHE_SIZE_BYTES,
            native_max_concurrent_compilations: DEFAULT_MAX_CONCURRENT_COMPILATIONS,
            native_compilation_timeout_secs: DEFAULT_COMPILATION_TIMEOUT_SECS,
            native_memory_cache_timeout_ms: DEFAULT_MEMORY_CACHE_TIMEOUT_MS,
            native_disk_cache_load_timeout_secs: DEFAULT_DISK_CACHE_LOAD_TIMEOUT_SECS,
            native_compilation_mode: "async".to_string(),
            native_enable_retry: true, // Retry enabled by default
            native_max_failed_compilations: DEFAULT_MAX_FAILED_COMPILATIONS,
        }
    }
}

impl CairoNativeParams {
    /// Returns the directory path where compiled classes are stored.
    ///
    /// The path is determined by checking (in order):
    /// 1. The `native_cache_dir` CLI parameter if set (or `MADARA_NATIVE_CACHE_DIR` env var via clap)
    /// 2. The default path from `DEFAULT_CACHE_DIR` constant
    pub fn cache_dir(&self) -> PathBuf {
        self.native_cache_dir.clone().unwrap_or_else(|| PathBuf::from(DEFAULT_CACHE_DIR))
    }

    /// Returns the compilation timeout as a `Duration` for use with async timers.
    pub fn compilation_timeout(&self) -> Duration {
        Duration::from_secs(self.native_compilation_timeout_secs)
    }

    /// Converts CLI parameters into the runtime configuration used by the compilation system.
    ///
    /// This performs the final transformation from user-facing CLI parameters to the internal
    /// configuration format. Call this once at startup.
    ///
    /// # Transformations
    ///
    /// - Compilation mode strings are normalized and converted to enum variants
    /// - Cache size values of "unlimited" are converted to `None` (no limit)
    /// - Default values are already applied, so this just handles format conversion
    ///
    /// # Validation
    ///
    /// Validation is performed automatically by `NativeConfig::validate()` which is called
    /// via `setup_and_log()` after conversion. This ensures configuration errors are caught
    /// before the system starts using the configuration.
    pub fn to_runtime_config(&self) -> mc_cairo_native::config::NativeConfig {
        use mc_cairo_native::config::NativeCompilationMode;

        // Normalize compilation mode: parse string and convert to enum
        let mode = match self.native_compilation_mode.to_lowercase().as_str() {
            "blocking" => NativeCompilationMode::Blocking,
            _ => NativeCompilationMode::Async, // Default to async for any unrecognized value
        };

        // Convert cache sizes: "unlimited" becomes None, numeric values become Some(n)
        let max_memory_cache_size = if self.native_max_memory_cache_classes == usize::MAX {
            None // User requested unlimited cache
        } else {
            Some(self.native_max_memory_cache_classes) // User specified a numeric limit
        };

        let max_disk_cache_size = if self.native_max_disk_cache_bytes == u64::MAX {
            None // User requested unlimited disk cache
        } else {
            Some(self.native_max_disk_cache_bytes) // User specified a numeric limit in bytes
        };

        mc_cairo_native::config::NativeConfig::new()
            .with_native_execution(self.enable_native_execution)
            .with_cache_dir(self.cache_dir())
            .with_max_memory_cache_size(max_memory_cache_size)
            .with_max_disk_cache_size(max_disk_cache_size)
            .with_max_concurrent_compilations(self.native_max_concurrent_compilations)
            .with_compilation_timeout(self.compilation_timeout())
            .with_memory_cache_timeout(Duration::from_millis(self.native_memory_cache_timeout_ms))
            .with_disk_cache_load_timeout(Duration::from_secs(self.native_disk_cache_load_timeout_secs))
            .with_compilation_mode(mode)
            .with_enable_retry(self.native_enable_retry)
            .with_max_failed_compilations(self.native_max_failed_compilations)
    }
}
