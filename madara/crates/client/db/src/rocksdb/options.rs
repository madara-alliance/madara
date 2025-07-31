#![allow(clippy::identity_op)] // allow 1 * MiB
#![allow(non_upper_case_globals)] // allow KiB/MiB/GiB names

use crate::rocksdb::column::{Column, ColumnMemoryBudget};
use anyhow::{Context, Result};
use rocksdb::{DBCompressionType, Env, Options, SliceTransform};

const KiB: usize = 1024;
const MiB: usize = 1024 * KiB;
const GiB: usize = 1024 * MiB;

pub use rocksdb::statistics::StatsLevel;

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
            snapshot_interval: 5
        }
    }
}

pub fn rocksdb_global_options(config: &RocksDBConfig) -> Result<Options> {
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    options.increase_parallelism(cores);
    options.set_max_background_jobs(cores);

    options.set_atomic_flush(true);
    options.set_max_subcompactions(cores as _);

    options.set_max_log_file_size(10 * MiB);
    options.set_max_open_files(2048);
    options.set_keep_log_file_num(3);
    options.set_log_level(rocksdb::LogLevel::Warn);

    if config.enable_statistics {
        options.enable_statistics();
        options.set_statistics_level(config.statistics_level);
    }
    options.set_stats_dump_period_sec(config.statistics_period_sec);

    let mut env = Env::new().context("Creating rocksdb env")?;
    env.set_low_priority_background_threads(cores); // compaction

    options.set_env(&env);

    Ok(options)
}

impl Column {
    /// Per column rocksdb options, like memory budget, compaction profiles, block sizes
    /// etc.
    pub(crate) fn rocksdb_options(&self, config: &RocksDBConfig) -> Options {
        // See column-specific options here:
        // https://github.com/facebook/rocksdb/blob/c237022831aa129aa707bc28e0702a1617ef23b5/include/rocksdb/advanced_options.h#L325
        // https://github.com/facebook/rocksdb/blob/c237022831aa129aa707bc28e0702a1617ef23b5/include/rocksdb/advanced_options.h#L148

        let mut options = Options::default();

        if let Some(prefix_extractor_len) = self.prefix_extractor_len {
            options.set_prefix_extractor(SliceTransform::create_fixed_prefix(prefix_extractor_len));
            options.set_memtable_prefix_bloom_ratio(config.memtable_prefix_bloom_filter_ratio);
        }

        options.set_compression_type(DBCompressionType::Zstd);
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

        if self.point_lookup {
            options.optimize_for_point_lookup(5); // 5 MiB
        }

        options
    }
}
