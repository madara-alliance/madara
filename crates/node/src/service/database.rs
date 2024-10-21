use anyhow::Context;
use mc_db::{DBDropHook, MadaraBackend};

use crate::cli::DbParams;

pub struct DatabaseService(DBDropHook);

impl DatabaseService {
    pub fn open(config: &DbParams) -> anyhow::Result<Self> {
        tracing::info!("💾 Opening database at: {}", config.base_path.display());

        let _madara_backend =
            MadaraBackend::open(&config.base_path, config.backup_dir.clone(), config.restore_from_latest_backup)
                .context("Opening database")?;

        Ok(Self(DBDropHook))
    }
}

#[async_trait::async_trait]
impl Service for DatabaseService {}
