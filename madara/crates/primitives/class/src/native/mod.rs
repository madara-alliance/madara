//! Cairo Native execution module
//!
//! This module provides Cairo Native execution as an alternative to Cairo VM for executing
//! Starknet contracts. It implements a multi-tier caching system with compilation management,
//! error handling, and comprehensive metrics tracking.
//!
//! **Important**: Native execution is only enabled when the `--enable-native-execution` CLI flag
//! is set to `true`. When disabled (default), all contracts use Cairo VM execution regardless
//! of cache state or compilation availability.
//!
//! # Module Organization
//!
//! - [`config`]: Configuration management for native execution settings
//! - [`error`]: Error types for compilation and execution failures
//! - [`metrics`]: Performance and operational metrics tracking
//! - [`cache`]: In-memory and disk-based caching of compiled classes
//! - [`compilation`]: Compilation orchestration (blocking/async modes, retry logic)
//! - [`execution`]: Main execution flow and class handling
//!
//! # Architecture Overview
//!
//! ## Execution Flow
//!
//! When a contract class is requested, the following flow is executed:
//!
//! ```text
//! Request for Class
//!       │
//!       ├─► Check In-Memory Cache (NATIVE_CACHE)
//!       │   ├─► Hit? → Return cached native class
//!       │   └─► Miss? → Continue
//!       │
//!       ├─► Check Disk Cache (.so files)
//!       │   ├─► Hit? → Load from disk, add to memory cache, return
//!       │   └─► Miss? → Continue
//!       │
//!       ├─► Check Compilation Mode
//!       │   │
//!       │   ├─► BLOCKING MODE:
//!       │   │   ├─► Compile synchronously (wait for completion)
//!       │   │   ├─► Success? → Cache and return native class
//!       │   │   └─► Failure? → Return error (panic upstream)
//!       │   │
//!       │   └─► ASYNC MODE:
//!       │       ├─► Check if already compiling → Use VM fallback
//!       │       ├─► Check if previously failed → Retry compilation
//!       │       ├─► Spawn background compilation task
//!       │       └─► Fall back to Cairo VM immediately
//!       │
//!       └─► Return Cairo VM execution (CASM)
//! ```
//!
//! See individual module documentation for more details.

pub mod cache;
pub mod compilation;
pub mod config;
pub mod error;
pub mod execution;
pub mod metrics;

pub use compilation::init_compilation_semaphore;
