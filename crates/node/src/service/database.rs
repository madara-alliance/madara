pub struct DatabaseService(DBDropHook);

impl DatabaseService {
    pub fn new(
        base_path: PathBuf,
        backup_dir: Option<PathBuf>,
        restore_from_latest_backup: bool,
    ) -> anyhow::Result<Self> {
        let deoxys_backend =
            DeoxysBackend::open(base_path, backup_dir, restore_from_latest_backup).context("opening database")?;

        Ok(Self(DBDropHook))
    }
}
