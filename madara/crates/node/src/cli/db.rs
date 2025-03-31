use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, clap::Args, Serialize, Deserialize)]
pub struct DbParams {
    /// The path where madara will store the database. You should probably change it.
    #[clap(env = "MADARA_BASE_PATH", long, default_value = "/tmp/madara", value_name = "PATH")]
    pub base_path: PathBuf,

    /// Directory for backups. Use it with `--restore-from-latest-backup` or `--backup-every-n-blocks <NUMBER OF BLOCKS>`.
    #[clap(env = "MADARA_BACKUP_DIR", long, value_name = "PATH")]
    pub backup_dir: Option<PathBuf>,

    /// Restore the database at startup from the latest backup version. Use it with `--backup-dir <PATH>`
    #[clap(env = "MADARA_RESTORE_FROM_LATEST_BACKUP", long)]
    pub restore_from_latest_backup: bool,

    /// This is the number of blocks for which you can get storage proofs using the storage proof endpoints.
    /// Blocks older than this limit will not be stored for retrieving historical merkle trie state. By default,
    /// the value 0 means that no historical merkle trie state access is allowed.
    #[clap(env = "MADARA_DB_MAX_SAVED_TRIE_LOGS", long, default_value_t = 0)]
    pub db_max_saved_trie_logs: usize,

    /// This affects the performance of the storage proof endpoint.
    /// How many databse snapshots are kept at a given time, older ones will be discarded.
    /// Snapshots are used to keep a view of the database in the past. They speed up reverting the global tries
    /// when getting a storage proof.
    /// Higher values cause more database space usage, while lower values prevent the efficient reverting and historical access for
    /// the global state trie at older blocks.
    #[clap(env = "MADARA_DB_MAX_SNAPSHOTS", long, default_value_t = 0)]
    pub db_max_kept_snapshots: usize,

    /// This affects the performance of the storage proof endpoint.
    /// A database snapshot is created every `db_snapshot_interval` blocks.
    /// See `--db-max-kept-snapshots` to understand what snapshots are used for.
    #[clap(env = "MADARA_DB_SNAPSHOT_INTERVAL", long, default_value_t = 5)]
    pub db_snapshot_interval: u64,
}
