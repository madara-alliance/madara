#![allow(clippy::identity_op)] // allow 1 * MiB
#![allow(non_upper_case_globals)] // allow KiB/MiB/GiB names

//! # RocksDB Configuration for Madara
//!
//! This module configures RocksDB, an embedded key-value store based on a Log-Structured
//! Merge Tree (LSM-Tree). Understanding the architecture is essential for tuning performance
//! and preventing write stalls.
//!
//! ## LSM-Tree Architecture Overview
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────────┐
//! │                                  MEMORY                                      │
//! ├──────────────────────────────────────────────────────────────────────────────┤
//! │                                                                              │
//! │   ┌─────────────────┐    ┌──────────────────┐    ┌──────────────────┐        │
//! │   │ Active MemTable │    │Immutable MemTable│    │Immutable MemTable│        │
//! │   │ (writes here)   │    │ (waiting flush)  │    │ (waiting flush)  │        │
//! │   └─────────────────┘    └──────────────────┘    └──────────────────┘        │
//! │          │                        │                      │                   │
//! │          │         max_write_buffer_number = 5           │                   │
//! │          │         (WRITE STALL if exceeded)             │                   │
//! │          │                        │                      │                   │
//! └──────────┼────────────────────────┼──────────────────────┼───────────────────┘
//!            │                        └──────────┬───────────┘
//!            │                                   │ FLUSH (background)
//!            │                                   ▼
//! ┌──────────┼───────────────────────────────────────────────────────────────────┐
//! │          │                     DISK (SST Files)                              │
//! ├──────────┼───────────────────────────────────────────────────────────────────┤
//! │          ▼                                                                   │
//! │  ┌───────────────────────────────────────────────────────────────────────┐   │
//! │  │ LEVEL 0 (L0) - Unsorted, overlapping SST files from memtable flushes  │   │
//! │  │ [SST][SST][SST][SST][SST]...                                          │   │
//! │  │                                                                       │   │
//! │  │ level_zero_slowdown_writes_trigger = 20  (start slowing writes)       │   │
//! │  │ level_zero_stop_writes_trigger = 36      (stop writes completely)     │   │
//! │  └───────────────────────────────────────────────────────────────────────┘   │
//! │                 │ COMPACTION (merge + sort + push down)                      │
//! │                 ▼                                                            │
//! │  ┌───────────────────────────────────────────────────────────────────────┐   │
//! │  │ LEVEL 1+ - Sorted, non-overlapping SST files                          │   │
//! │  │ Each level is ~10x larger than the previous                           │   │
//! │  │                                                                       │   │
//! │  │ soft_pending_compaction_bytes = 6 GiB  (slow writes)                  │   │
//! │  │ hard_pending_compaction_bytes = 12 GiB (stop writes)                  │   │
//! │  └───────────────────────────────────────────────────────────────────────┘   │
//! └──────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Write Path
//!
//! 1. **WAL Write**: Data is first written to Write-Ahead Log (sequential, fast)
//! 2. **MemTable Insert**: Data inserted into in-memory sorted structure (SkipList)
//! 3. **Return Success**: Write is "committed" - very fast
//! 4. **Background Flush**: When memtable fills, it's flushed to L0 as SST file
//! 5. **Compaction**: Background threads merge and push data to deeper levels
//!
//! ## Write Stall Conditions
//!
//! RocksDB stalls writes to prevent unbounded memory/disk growth when background
//! operations can't keep up:
//!
//! | Condition | Trigger | Effect |
//! |-----------|---------|--------|
//! | Too many memtables | `count >= max_write_buffer_number` | Writes blocked |
//! | Too many L0 files | `count >= level_zero_slowdown_writes_trigger` | Writes slowed |
//! | Too many L0 files | `count >= level_zero_stop_writes_trigger` | Writes blocked |
//! | Pending compaction | `bytes >= soft_pending_compaction_bytes` | Writes slowed |
//! | Pending compaction | `bytes >= hard_pending_compaction_bytes` | Writes blocked |
//!
//! ## Key Configuration Categories
//!
//! ### 1. MemTable Settings
//! - `write_buffer_size`: Size of single memtable (larger = fewer flushes, more memory)
//! - `max_write_buffer_number`: Max memtables before stall (higher = more buffer)
//! - `min_write_buffer_number_to_merge`: Memtables to merge before flush (reduces L0 files)
//!
//! ### 2. L0 Thresholds
//! - `level_zero_slowdown_writes_trigger`: Start throttling (gradual slowdown)
//! - `level_zero_stop_writes_trigger`: Complete stop (hard limit)
//!
//! ### 3. Compaction Limits
//! - `soft_pending_compaction_bytes_limit`: Soft limit for pending compaction
//! - `hard_pending_compaction_bytes_limit`: Hard limit (writes stop)
//!
//! ### 4. I/O Smoothing
//! - `bytes_per_sync`: Incremental sync during SST writes (prevents I/O spikes)
//!
//! ## References
//!
//! - [RocksDB Tuning Guide](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide)
//! - [Write Stalls](https://github.com/facebook/rocksdb/wiki/Write-Stalls)
//! - [TiKV RocksDB Config](https://tikv.org/docs/3.0/tasks/configure/rocksdb/)

use std::path::PathBuf;

use crate::rocksdb::column::{Column, ColumnMemoryBudget};
use anyhow::{Context, Result};
use rocksdb::{DBCompressionType, Env, Options, SliceTransform, WriteOptions};
use serde::{Deserialize, Serialize};

const KiB: usize = 1024;
const MiB: usize = 1024 * KiB;
const GiB: usize = 1024 * MiB;

pub use rocksdb::statistics::StatsLevel;

/// RocksDB write durability configuration. Controls WAL (Write-Ahead Log) and fsync behavior.
///
/// # Write Path with WAL
///
/// ```text
/// Application ──► WAL (disk) ──► MemTable (memory) ──► Return
///                  │
///                  └── Sequential writes, very fast
/// ```
///
/// # Performance & Safety Trade-offs
///
/// | WAL     | Fsync   | Performance | Safety  | Use Case                              |
/// |---------|---------|-------------|---------|---------------------------------------|
/// | Enabled | Enabled | Slowest     | Highest | Production critical data              |
/// | Enabled | Disable | Medium      | High    | Production (safe on crash) - **RECOMMENDED** |
/// | Disable | Enabled | Fast        | Medium  | Testing, can tolerate data loss       |
/// | Disable | Disable | Fastest     | Lowest  | Devnet, testing                       |
///
/// # Safety Considerations
///
/// - **WAL enabled**: Write-Ahead Log records changes before applying them. Enables crash recovery.
///   Disabling WAL is faster but may lose recent uncommitted data on crash.
///
/// - **Fsync enabled**: Forces data to be flushed to disk before acknowledging writes.
///   Survives power failures but slower. Disabling fsync relies on OS buffering (faster, survives crashes but not power loss).
///
/// **Recommended settings:**
/// - Production: `wal=true, fsync=false` (safe on crash, good performance)
/// - Maximum durability: `wal=true, fsync=true` (survives power failures)
/// - Testing/Development: `wal=false, fsync=false` (fastest)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbWriteMode {
    /// Enable Write-Ahead Log (WAL). Provides crash recovery but adds overhead.
    /// Default: true (recommended for production)
    pub wal: bool,
    /// Enable fsync after writes. Ensures data reaches disk before acknowledging.
    /// Default: false (recommended for production - survives crashes, faster than fsync)
    pub fsync: bool,
}

impl Default for DbWriteMode {
    fn default() -> Self {
        Self { wal: true, fsync: false }
    }
}

impl DbWriteMode {
    /// Convert the write mode to RocksDB WriteOptions
    pub fn to_write_options(&self) -> WriteOptions {
        let mut opts = WriteOptions::default();

        if !self.wal {
            opts.disable_wal(true);
        }

        if !self.fsync {
            opts.set_sync(false);
        }

        opts
    }
}

impl std::fmt::Display for DbWriteMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.wal, self.fsync) {
            (true, true) => write!(f, "WAL enabled, fsync enabled (safest)"),
            (true, false) => write!(f, "WAL enabled, fsync disabled (recommended)"),
            (false, true) => write!(f, "WAL disabled, fsync enabled (fast)"),
            (false, false) => write!(f, "WAL disabled, fsync disabled (fastest, least safe)"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RocksDBConfig {
    /// Enable statistics. Statistics will be put in the `LOG` file in the db folder. This can have an effect on performance.
    pub enable_statistics: bool,
    /// Dump statistics every `statistics_period_sec`.
    pub statistics_period_sec: u32,
    /// Statistics level. This can have an effect on performance.
    pub statistics_level: StatsLevel,
    /// Memory budget for blocks-related columns
    pub memtable_blocks_budget_bytes: usize,
    /// Memory budget for contracts-related columns
    pub memtable_contracts_budget_bytes: usize,
    /// Memory budget for other columns
    pub memtable_other_budget_bytes: usize,
    /// Ratio of the buffer size dedicated to bloom filters for a column
    pub memtable_prefix_bloom_filter_ratio: f64,

    /// Maximum number of trie logs
    pub max_saved_trie_logs: Option<usize>,
    /// Maximum number of kept snapshots
    pub max_kept_snapshots: Option<usize>,
    /// Number of blocks between snapshots
    pub snapshot_interval: u64,

    /// When present, every flush will create a backup.
    pub backup_dir: Option<PathBuf>,
    /// When true, the latest backup will be restored on startup.
    pub restore_from_latest_backup: bool,

    /// Write durability mode (WAL and fsync settings)
    pub write_mode: DbWriteMode,

    // ═══════════════════════════════════════════════════════════════════════════
    // WRITE STALL PREVENTION SETTINGS
    // ═══════════════════════════════════════════════════════════════════════════
    /// Maximum number of memtables (active + immutable) before write stall.
    /// Higher values provide more buffer during write bursts but use more memory.
    /// Default: 5 (allows 4 immutable memtables to queue while flushing)
    pub max_write_buffer_number: i32,

    /// Number of L0 files that triggers write slowdown.
    /// When L0 file count reaches this, writes are throttled.
    /// Default: 20
    pub level_zero_slowdown_writes_trigger: i32,

    /// Number of L0 files that triggers complete write stop.
    /// When L0 file count reaches this, writes are blocked until compaction catches up.
    /// Default: 36
    pub level_zero_stop_writes_trigger: i32,

    /// Soft limit for pending compaction bytes. When exceeded, writes slow down.
    pub soft_pending_compaction_bytes_limit: usize,

    /// Hard limit for pending compaction bytes. When exceeded, writes stop completely.
    pub hard_pending_compaction_bytes_limit: usize,
}

impl Default for RocksDBConfig {
    fn default() -> Self {
        Self {
            enable_statistics: false,
            statistics_period_sec: 60,
            statistics_level: StatsLevel::All,
            // TODO: these might not be the best defaults at all
            memtable_blocks_budget_bytes: 1 * GiB,
            memtable_contracts_budget_bytes: 128 * MiB,
            memtable_other_budget_bytes: 128 * MiB,
            memtable_prefix_bloom_filter_ratio: 0.0,
            max_saved_trie_logs: None,
            max_kept_snapshots: None,
            snapshot_interval: 5,
            backup_dir: None,
            restore_from_latest_backup: false,
            write_mode: DbWriteMode::default(),
            // Write stall prevention defaults (tuned for ~20 GiB volumes)
            max_write_buffer_number: 5,
            level_zero_slowdown_writes_trigger: 20,
            level_zero_stop_writes_trigger: 36,
            soft_pending_compaction_bytes_limit: 6 * GiB,  // ~30% of 20 GiB
            hard_pending_compaction_bytes_limit: 12 * GiB, // ~60% of 20 GiB
        }
    }
}

/// Configure global RocksDB options for optimal performance and write stall prevention.
///
/// # Architecture Impact
///
/// This function configures options that affect the entire RocksDB instance:
///
/// ## Background Jobs & Parallelism
///
/// ```text
/// ┌──────────────────────────────────────────────────────────────────┐
/// │  Thread Pool (max_background_jobs = CPU cores)                   │
/// │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐     │
/// │  │ Flush   │ │Compact  │ │Compact  │ │Compact  │ │Compact  │     │
/// │  │ Thread  │ │Thread 1 │ │Thread 2 │ │Thread 3 │ │Thread 4 │     │
/// │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘     │
/// └──────────────────────────────────────────────────────────────────┘
/// ```
///
/// ## Write Stall Prevention
///
/// ### MemTable Configuration
///
/// ```text
/// max_write_buffer_number = 5
///
/// ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
/// │  Active  │ │ Immut #1 │ │ Immut #2 │ │ Immut #3 │ │ Immut #4 │
/// │MemTable  │ │(flushing)│ │(waiting) │ │(waiting) │ │(waiting) │
/// └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘
///      │
///      │  With max=5, we can queue 4 immutable memtables
///      │  This prevents stalls when flush is slow
///      ▼
/// ```
///
/// ### L0 File Thresholds
///
/// ```text
/// L0: [SST][SST][SST]...[SST][SST][SST]
///      1    2    3  ...  18   19   20
///                              │    │
///                              │    └─ level_zero_slowdown_writes_trigger = 20
///                              │       (writes throttled to delayed_write_rate)
///                              │
///      At 36 files ────────────┴────── level_zero_stop_writes_trigger = 36
///                                      (writes completely blocked)
/// ```
///
/// ### Pending Compaction Limits
///
/// Note: Defaults (6/12 GiB) are tuned for ~20 GiB volumes. Increase for
/// production databases with higher write traffic.
///
/// ```text
/// Pending Compaction Bytes
/// ════════════════════════
///
/// 0 GB ───────────────────────────────────────► ∞
///      │              │              │
///      │              │              │
///   Normal          6 GB           12 GB
///   Speed     (soft limit:    (hard limit:
///              slowdown)        STOP)
/// ```
///
/// ### I/O Smoothing with bytes_per_sync
///
/// ```text
/// Without bytes_per_sync (default=0):
/// [write 256MB file] ────────────────────────────► [single huge sync]
///                                                        ↑
///                                            I/O spike, potential stall
///
/// With bytes_per_sync=1MB:
/// [write] ► [sync] ► [write] ► [sync] ► [write] ► [sync] ...
///           1MB       1MB       1MB       1MB
///           Smooth I/O, no spikes
/// ```
pub fn rocksdb_global_options(config: &RocksDBConfig) -> Result<Options> {
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);

    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);

    // ═══════════════════════════════════════════════════════════════════════════
    // PARALLELISM CONFIGURATION
    // ═══════════════════════════════════════════════════════════════════════════
    // Controls background thread pool for flush and compaction operations.
    // More threads = faster background work = less likely to hit write stalls.
    options.increase_parallelism(cores);
    options.set_max_background_jobs(cores);

    // Atomic flush ensures all column families are flushed together,
    // maintaining consistency across the database.
    options.set_atomic_flush(true);

    // Allow compaction jobs to be split across multiple threads.
    // A single large compaction can use up to `cores` threads.
    options.set_max_subcompactions(cores as _);

    // ═══════════════════════════════════════════════════════════════════════════
    // WRITE STALL PREVENTION - MEMTABLE SETTINGS
    // ═══════════════════════════════════════════════════════════════════════════
    // These settings control when RocksDB stalls writes due to memtable pressure.
    //
    // Default max_write_buffer_number=2 is too aggressive for high-throughput
    // blockchain workloads. With only 2 buffers, if the active memtable fills
    // while the immutable one is still flushing, writes stall immediately.
    //
    // Setting to 5 allows 4 immutable memtables to queue while flushing,
    // providing much more headroom during write bursts.
    options.set_max_write_buffer_number(config.max_write_buffer_number);

    // Merge 2 memtables before flushing to L0.
    // This reduces L0 file count (fewer files = less read amplification)
    // and removes duplicate keys, reducing overall data size.
    options.set_min_write_buffer_number_to_merge(2);

    // ═══════════════════════════════════════════════════════════════════════════
    // WRITE STALL PREVENTION - L0 THRESHOLDS
    // ═══════════════════════════════════════════════════════════════════════════
    // L0 (Level 0) is special: files can have overlapping key ranges.
    // Too many L0 files = slow reads (must check all files).
    // RocksDB stalls writes to prevent unbounded L0 growth.
    //
    // Slowdown: Writes are artificially delayed (throttled to delayed_write_rate).
    // This gives compaction time to catch up without completely blocking.
    options.set_level_zero_slowdown_writes_trigger(config.level_zero_slowdown_writes_trigger);

    // Stop: Writes are completely blocked until compaction reduces L0 files.
    // This is the "emergency brake" to prevent runaway disk usage.
    options.set_level_zero_stop_writes_trigger(config.level_zero_stop_writes_trigger);

    // ═══════════════════════════════════════════════════════════════════════════
    // WRITE STALL PREVENTION - PENDING COMPACTION LIMITS
    // ═══════════════════════════════════════════════════════════════════════════
    // When compaction falls behind, uncompacted data accumulates.
    // These limits prevent unbounded growth of pending compaction work.
    //
    // Soft limit: Start slowing writes when pending compaction exceeds the limit.
    options.set_soft_pending_compaction_bytes_limit(config.soft_pending_compaction_bytes_limit);

    // Hard limit: Stop writes completely when pending compaction is too high.
    options.set_hard_pending_compaction_bytes_limit(config.hard_pending_compaction_bytes_limit);

    // ═══════════════════════════════════════════════════════════════════════════
    // I/O SMOOTHING
    // ═══════════════════════════════════════════════════════════════════════════
    // Without incremental sync, RocksDB writes entire SST files (potentially
    // hundreds of MB) before syncing, causing I/O spikes that can stall writes.
    //
    // bytes_per_sync: Sync every 1MB during SST file writes.
    // This spreads I/O evenly, preventing spikes.
    options.set_bytes_per_sync(1 * MiB as u64);

    // wal_bytes_per_sync: Sync WAL every 512KB.
    // WAL writes are smaller but more frequent; smaller sync interval is appropriate.
    options.set_wal_bytes_per_sync(512 * KiB as u64);

    // Note: enable_pipelined_write is NOT used because it's incompatible with
    // atomic_flush, which we need for cross-column-family consistency.

    // ═══════════════════════════════════════════════════════════════════════════
    // LOGGING & FILE MANAGEMENT
    // ═══════════════════════════════════════════════════════════════════════════
    options.set_max_log_file_size(10 * MiB);
    options.set_max_open_files(2048);
    options.set_keep_log_file_num(3);
    options.set_log_level(rocksdb::LogLevel::Warn);

    // ═══════════════════════════════════════════════════════════════════════════
    // STATISTICS (Optional, for debugging)
    // ═══════════════════════════════════════════════════════════════════════════
    // Statistics add overhead but are invaluable for diagnosing performance issues.
    // Enable temporarily with --db-enable-statistics to debug stalls.
    if config.enable_statistics {
        options.enable_statistics();
        options.set_statistics_level(config.statistics_level);
    }
    options.set_stats_dump_period_sec(config.statistics_period_sec);

    // ═══════════════════════════════════════════════════════════════════════════
    // ENVIRONMENT CONFIGURATION
    // ═══════════════════════════════════════════════════════════════════════════
    let mut env = Env::new().context("Creating rocksdb env")?;
    // Low priority threads are used for compaction (can be preempted by flush).
    env.set_low_priority_background_threads(cores);

    options.set_env(&env);

    Ok(options)
}

impl Column {
    /// Configure per-column RocksDB options.
    ///
    /// # Column-Specific Tuning
    ///
    /// Each column family can have different settings based on its access patterns:
    ///
    /// ## Memory Budget Tiers
    ///
    /// ```text
    /// ┌─────────────────────────────────────────────────────────────────┐
    /// │  Blocks Tier (1 GiB)     - High-frequency block data writes     │
    /// │  Contracts Tier (128 MiB) - State updates, nonces, storage      │
    /// │  Other Tier (128 MiB)    - Metadata, classes, events            │
    /// └─────────────────────────────────────────────────────────────────┘
    /// ```
    ///
    /// ## Compaction Strategy
    ///
    /// We use Universal Compaction which:
    /// - Has lower write amplification than Level Compaction
    /// - Better for write-heavy workloads (like blockchain sync)
    /// - Trade-off: Higher space amplification
    ///
    /// ## Compression
    ///
    /// Zstd compression provides excellent compression ratio with good speed.
    /// This reduces disk I/O and storage requirements.
    pub(crate) fn rocksdb_options(&self, config: &RocksDBConfig) -> Options {
        // See column-specific options here:
        // https://github.com/facebook/rocksdb/blob/c237022831aa129aa707bc28e0702a1617ef23b5/include/rocksdb/advanced_options.h#L325
        // https://github.com/facebook/rocksdb/blob/c237022831aa129aa707bc28e0702a1617ef23b5/include/rocksdb/advanced_options.h#L148

        let mut options = Options::default();

        // Prefix extractor enables efficient prefix scans.
        // Used for columns that frequently query by prefix (e.g., block transactions).
        if let Some(prefix_extractor_len) = self.prefix_extractor_len {
            options.set_prefix_extractor(SliceTransform::create_fixed_prefix(prefix_extractor_len));
            options.set_memtable_prefix_bloom_ratio(config.memtable_prefix_bloom_filter_ratio);
        }

        // Zstd provides excellent compression with good read/write performance.
        options.set_compression_type(DBCompressionType::Zstd);

        // Configure memory budget and compaction based on column tier.
        // optimize_universal_style_compaction sets:
        // - write_buffer_size (memtable size)
        // - Universal compaction settings
        // - Appropriate L0 thresholds for the budget
        match self.budget_tier {
            ColumnMemoryBudget::Blocks => {
                options.set_memtable_prefix_bloom_ratio(config.memtable_prefix_bloom_filter_ratio);
                options.optimize_universal_style_compaction(config.memtable_blocks_budget_bytes);
            }
            ColumnMemoryBudget::Contracts => {
                options.set_memtable_prefix_bloom_ratio(config.memtable_prefix_bloom_filter_ratio);
                options.optimize_universal_style_compaction(config.memtable_contracts_budget_bytes);
            }
            ColumnMemoryBudget::Other => {
                options.set_memtable_prefix_bloom_ratio(config.memtable_prefix_bloom_filter_ratio);
                options.optimize_universal_style_compaction(config.memtable_other_budget_bytes);
            }
        }

        // Point lookup optimization for columns that primarily do single-key gets.
        // Adds a bloom filter in the block cache for faster negative lookups.
        if self.point_lookup {
            options.optimize_for_point_lookup(5); // 5 MiB bloom filter cache
        }

        options
    }
}
