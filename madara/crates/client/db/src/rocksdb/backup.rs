use crate::{
    prelude::*,
    rocksdb::{RocksDBConfig, RocksDBStorageInner},
};
use anyhow::Context;
use rocksdb::{
    backup::{BackupEngine, BackupEngineOptions},
    Env,
};
use std::{fs, path::Path, sync::Arc};

#[derive(Debug, Clone)]
pub struct BackupManager(Option<std::sync::mpsc::Sender<BackupRequest>>);

impl BackupManager {
    pub fn start_if_enabled(db_path: &Path, config: &RocksDBConfig) -> Result<Self> {
        // when backups are enabled, a thread is spawned that owns the rocksdb BackupEngine (it is not thread safe) and it receives backup requests using an mpsc channel
        // There is also another oneshot channel involved: when restoring the db at startup, we want to wait for the backupengine to finish restorating before returning from open()
        let Some(backup_dir) = config.backup_dir.clone() else { return Ok(Self(None)) };

        let (restored_cb_sender, restored_cb_recv) = std::sync::mpsc::channel();

        let (sender, receiver) = std::sync::mpsc::channel();
        let (db_path, restore_from_latest_backup) = (db_path.to_owned(), config.restore_from_latest_backup);
        std::thread::spawn(move || {
            Self::spawn(&backup_dir, restore_from_latest_backup, &db_path, restored_cb_sender, receiver)
                .expect("Database backup thread")
        });

        tracing::debug!("blocking on db restoration");
        restored_cb_recv.recv().context("Restoring database")?;
        tracing::debug!("done blocking on db restoration");

        Ok(Self(Some(sender)))
    }

    /// This runs in another thread as the backup engine is not thread safe
    fn spawn(
        backup_dir: &Path,
        restore_from_latest_backup: bool,
        db_path: &Path,
        db_restored_cb: std::sync::mpsc::Sender<()>,
        recv: std::sync::mpsc::Receiver<BackupRequest>,
    ) -> anyhow::Result<()> {
        let mut backup_opts = BackupEngineOptions::new(backup_dir).context("Creating backup options")?;
        let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
        backup_opts.set_max_background_operations(cores);

        let mut engine = BackupEngine::open(&backup_opts, &Env::new().context("Creating rocksdb env")?)
            .context("Opening backup engine")?;

        if restore_from_latest_backup {
            tracing::info!("⏳ Restoring latest backup...");
            tracing::debug!("restore path is {db_path:?}");
            fs::create_dir_all(db_path).with_context(|| format!("Creating parent directories {:?}", db_path))?;

            let opts = rocksdb::backup::RestoreOptions::default();
            engine.restore_from_latest_backup(db_path, db_path, &opts).context("Restoring database")?;
            tracing::debug!("restoring latest backup done");
        }

        let _res = db_restored_cb.send(());

        while let Ok(BackupRequest { callback, db }) = recv.recv() {
            engine.create_new_backup_flush(&db.db, true).context("Creating rocksdb backup")?;
            let _res = callback.send(());
        }

        Ok(())
    }

    pub fn backup_if_enabled(&self, db: &Arc<RocksDBStorageInner>) -> Result<()> {
        let Some(sender) = self.0.as_ref() else { return Ok(()) };

        let (callback_sender, callback_recv) = std::sync::mpsc::channel();
        let _res = sender.send(BackupRequest { callback: callback_sender, db: db.clone() });
        callback_recv.recv().context("Backups task died :(")?;
        Ok(())
    }
}

struct BackupRequest {
    callback: std::sync::mpsc::Sender<()>,
    db: Arc<RocksDBStorageInner>,
}
