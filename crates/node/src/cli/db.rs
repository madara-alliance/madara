use std::path::PathBuf;

#[derive(Clone, Debug, clap::Args)]
pub struct DbParams {
    /// The base path.
    #[clap(long, default_value = "/tmp/deoxys")]
    pub base_path: PathBuf,

    #[clap(long)]
    pub backup_dir: Option<PathBuf>,

    #[clap(long)]
    pub restore_from_latest_backup: bool,
}
