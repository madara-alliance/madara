use mc_db::rocksdb::{DbWriteMode, RocksDBConfig};
use mc_db::MadaraBackendConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[allow(non_upper_case_globals)]
const KiB: usize = 1024;
#[allow(non_upper_case_globals)]
const MiB: usize = 1024 * KiB;

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Deserialize, Serialize)]
pub enum StatsLevel {
    /// Disable all metrics
    DisableAll,
    /// Disable timer stats, and skip histogram stats
    ExceptHistogramOrTimers,
    /// Skip timer stats
    ExceptTimers,
    /// Collect all stats except time inside mutex lock AND time spent on
    /// compression.
    ExceptDetailedTimers,
    /// Collect all stats except the counters requiring to get time inside the
    /// mutex lock.
    ExceptTimeForMutex,
    /// Collect all stats, including measuring duration of mutex operations.
    /// If getting time is expensive on the platform to run, it can
    /// reduce scalability to more threads, especially for writes.
    All,
}

impl From<StatsLevel> for mc_db::rocksdb::StatsLevel {
    fn from(value: StatsLevel) -> Self {
        match value {
            StatsLevel::DisableAll => Self::DisableAll,
            StatsLevel::ExceptHistogramOrTimers => Self::ExceptHistogramOrTimers,
            StatsLevel::ExceptTimers => Self::ExceptTimers,
            StatsLevel::ExceptDetailedTimers => Self::ExceptDetailedTimers,
            StatsLevel::ExceptTimeForMutex => Self::ExceptTimeForMutex,
            StatsLevel::All => Self::All,
        }
    }
}

#[derive(Clone, Debug, clap::Args, Deserialize, Serialize)]
pub struct BackendParams {
    /// The path where madara will store the database. You should probably change it.
    #[clap(env = "MADARA_BASE_PATH", long, default_value = "/tmp/madara", value_name = "PATH")]
    pub base_path: PathBuf,

    /// Directory for backups. Use it with `--restore-from-latest-backup` or `--backup-every-n-blocks <NUMBER OF BLOCKS>`.
    #[clap(env = "MADARA_BACKUP_DIR", long, value_name = "PATH")]
    pub backup_dir: Option<PathBuf>,

    /// Disable saving the preconfirmed block to database. This may speed up block production a bit.
    #[clap(env = "MADARA_NO_SAVE_PRECONFIRMED", long)]
    #[serde(default)]
    pub no_save_preconfirmed: bool,

    /// Skip creating backup before database migration.
    /// WARNING: Without backup, there's no recovery if migration fails.
    /// Only use if you have external snapshots/backups.
    #[clap(env = "MADARA_SKIP_MIGRATION_BACKUP", long)]
    #[serde(default)]
    pub skip_migration_backup: bool,

    /// Restore the database at startup from the latest backup version. Use it with `--backup-dir <PATH>`
    #[clap(env = "MADARA_RESTORE_FROM_LATEST_BACKUP", long)]
    pub restore_from_latest_backup: bool,

    /// This is the number of blocks for which you can get storage proofs using the storage proof endpoints.
    /// Blocks older than this limit will not be stored for retrieving historical merkle trie state. By default,
    /// the historical merkle trie state access is limited to 100 blocks by default.
    #[clap(env = "MADARA_DB_MAX_SAVED_TRIE_LOGS", long, default_value = Some("100"))]
    pub db_max_saved_trie_logs: Option<usize>,

    /// This affects the performance of the storage proof endpoint.
    /// How many databse snapshots are kept at a given time, older ones will be discarded.
    /// Snapshots are used to keep a view of the database in the past. They speed up reverting the global tries
    /// when getting a storage proof.
    /// Higher values cause more database space usage, while lower values prevent the efficient reverting and historical access for
    /// the global state trie at older blocks.
    #[clap(env = "MADARA_DB_MAX_SNAPSHOTS", long)]
    pub db_max_kept_snapshots: Option<usize>,

    /// This affects the performance of the storage proof endpoint.
    /// A database snapshot is created every `db_snapshot_interval` blocks.
    /// See `--db-max-kept-snapshots` to understand what snapshots are used for.
    #[clap(env = "MADARA_DB_SNAPSHOT_INTERVAL", long, default_value_t = 5)]
    pub db_snapshot_interval: u64,

    /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    #[clap(env = "MADARA_BACKUP_EVERY_N_BLOCKS", long, value_name = "NUMBER OF BLOCKS")]
    pub backup_every_n_blocks: Option<u64>,

    /// Periodically flushes the database from ram to disk based on the number
    /// of blocks synchronized since the last flush. You can set this to a
    /// higher number depending on how fast your machine is at synchronizing
    /// blocks and how much ram it has available.
    ///
    /// Note that keeping this value high could lead to blocks being stored in
    /// ram for longer periods of time before they are written to disk. This
    /// might be an issue for chains which synchronize slowly.
    #[clap(env = "MADARA_FLUSH_EVERY_N_BLOCKS", long, value_name = "NUMBER OF BLOCKS")]
    pub flush_every_n_blocks: Option<u64>,

    /// Enable rocksdb statistics. This has a small performance cost for every database operation.
    /// Statistics are dumped into the `LOG` file in the rocksdb database directory.
    #[clap(env = "MADARA_DB_ENABLE_STATISTICS", long)]
    pub db_enable_statistics: bool,

    /// If not zero, the rocksdb statistics will be dumped into the db LOG file with this frequency.
    /// The argument `--db-enable-statistics` is needed for this argument to have an effect.
    #[clap(env = "MADARA_DB_STATISTICS_PERIOD_SEC", long, default_value_t = 60)]
    pub db_statistics_period_sec: u32,

    /// Level of statistics. Collection all statistics may have a performance hit.
    /// The argument `--db-enable-statistics` is needed for this argument to have an effect.
    #[clap(env = "MADARA_DB_STATISTICS_LEVEL", long)]
    pub db_statistics_level: Option<StatsLevel>,

    /// Set the memtable budget for a set of columns.
    #[clap(env = "MADARA_DB_MEMTABLE_BLOCKS_BUDGET_MIB", long, default_value_t = 1024)]
    pub db_memtable_blocks_budget_mib: usize,

    /// Set the memtable budget for a set of columns.
    #[clap(env = "MADARA_DB_MEMTABLE_CONTRACTS_BUDGET_MIB", long, default_value_t = 128)]
    pub db_memtable_contracts_budget_mib: usize,

    /// Set the memtable budget for a set of columns.
    #[clap(env = "MADARA_DB_MEMTABLE_OTHER_BUDGET_MIB", long, default_value_t = 128)]
    pub db_memtable_other_budget_mib: usize,

    /// Set the rocksdb prefix bloom filter ratio.
    #[clap(env = "MADARA_DB_MEMTABLE_PREFIX_BLOOM_FILTER_RATIO", long, default_value_t = 0.0)]
    pub db_memtable_prefix_bloom_filter_ratio: f64,

    /// The block you want to start syncing from. This will most probably break your database.
    #[clap(env = "MADARA_UNSAFE_STARTING_BLOCK", long, value_name = "BLOCK NUMBER")]
    pub unsafe_starting_block: Option<u64>,

    /// Enable Write-Ahead Log (WAL) for database writes.
    /// WAL provides crash recovery by logging changes before applying them.
    /// Disabling improves performance but may lose recent data on crash.
    /// Recommended: true for production, false for testing/development.
    #[clap(env = "MADARA_DB_WAL", long, default_value = "true")]
    pub db_wal: bool,

    /// Enable fsync for database writes.
    /// Fsync forces data to disk before acknowledging writes, surviving power failures.
    /// Disabling relies on OS buffering (faster, survives crashes but not power loss).
    /// Recommended: false for production (good balance), true for maximum durability.
    #[clap(env = "MADARA_DB_FSYNC", long, default_value = "false")]
    pub db_fsync: bool,
}

impl BackendParams {
    pub fn backend_config(&self) -> MadaraBackendConfig {
        MadaraBackendConfig {
            flush_every_n_blocks: self.flush_every_n_blocks,
            save_preconfirmed: !self.no_save_preconfirmed,
            unsafe_starting_block: self.unsafe_starting_block,
            skip_migration_backup: self.skip_migration_backup,
        }
    }
    pub fn rocksdb_config(&self) -> RocksDBConfig {
        RocksDBConfig {
            enable_statistics: self.db_enable_statistics,
            statistics_period_sec: self.db_statistics_period_sec,
            statistics_level: self.db_statistics_level.unwrap_or(StatsLevel::All).into(),
            memtable_blocks_budget_bytes: self.db_memtable_blocks_budget_mib * MiB,
            memtable_contracts_budget_bytes: self.db_memtable_contracts_budget_mib * MiB,
            memtable_other_budget_bytes: self.db_memtable_other_budget_mib * MiB,
            memtable_prefix_bloom_filter_ratio: self.db_memtable_prefix_bloom_filter_ratio,
            max_saved_trie_logs: self.db_max_saved_trie_logs,
            max_kept_snapshots: self.db_max_kept_snapshots,
            snapshot_interval: self.db_snapshot_interval,
            backup_dir: self.backup_dir.clone(),
            restore_from_latest_backup: self.restore_from_latest_backup,
            write_mode: DbWriteMode { wal: self.db_wal, fsync: self.db_fsync },
        }
    }
}
