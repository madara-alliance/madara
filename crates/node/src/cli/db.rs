use std::path::PathBuf;
use serde::Deserialize;

#[derive(Clone, Debug, clap::Args, Deserialize)]
#[serde(default)]
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
}

impl Default for DbParams {
    fn default() -> Self {
        Self {
            base_path: "/tmp/madara".into(),
            backup_dir: None,
            restore_from_latest_backup: false,
        }
    }
}
