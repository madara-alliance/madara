use anyhow::Context;
use mc_db::{DBDropHook, DeoxysBackend};

use crate::cli::DbParams;

pub struct DatabaseService(DBDropHook);

impl DatabaseService {
    pub fn open(config: &DbParams) -> anyhow::Result<Self> {
        log::info!("💾 Opening database at: {}", config.base_path.display());

        let _deoxys_backend =
            DeoxysBackend::open(&config.base_path, config.backup_dir.clone(), config.restore_from_latest_backup)
                .context("opening database")?;

        Ok(Self(DBDropHook))
    }
}
