use std::path::PathBuf;

#[derive(Clone, Debug, clap::Args)]
pub struct DbParams {
    #[clap(long)]
    pub base_path: Option<PathBuf>,

    #[clap(long)]
    pub backup_dir: Option<PathBuf>,

    #[clap(long)]
    pub restore_from_latest_backup: bool,
}

impl DbParams {
    pub fn build(self) -> anyhow::Result<DatabaseService> {
        todo!()
    }
}