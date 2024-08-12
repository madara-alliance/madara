use std::path::PathBuf;

#[derive(Clone, Debug, clap::Args)]
pub struct DbParams {
    /// The path where madara will store the database. You should probably change it.
    #[clap(long, default_value = "/tmp/madara", value_name = "PATH")]
    pub base_path: PathBuf,

    /// Directory for backups. Use it with `--restore-from-latest-backup` or `--backup-every-n-blocks <NUMBER OF BLOCKS>`.
    #[clap(long, value_name = "PATH")]
    pub backup_dir: Option<PathBuf>,

    /// Restore the database at startup from the latest backup version. Use it with `--backup-dir <PATH>`
    #[clap(long)]
    pub restore_from_latest_backup: bool,
}
