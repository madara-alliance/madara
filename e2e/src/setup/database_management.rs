// =============================================================================
// DATABASE MANAGEMENT
// =============================================================================

use fs_extra::dir::{copy, CopyOptions};
use std::path::PathBuf;
use tokio::fs;
// Import all the services we've created
pub use super::config::*;
use crate::services::{constants::*, helpers::get_file_path};

pub struct DatabaseManager {}

impl DatabaseManager {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn create_data_directory(&self, dir_name: &str) -> Result<(), SetupError> {
        let data_dump_dir = REPO_ROOT.join(dir_name);
        if !data_dump_dir.exists() {
            fs::create_dir_all(data_dump_dir).await.map_err(|e| SetupError::OtherError(e.to_string()))?;
        }
        Ok(())
    }

    pub fn check_data_directory(&self, dir_name: &str) -> Result<(), SetupError> {
        let data_dump_dir = REPO_ROOT.join(dir_name);
        if !data_dump_dir.exists() {
            return Err(SetupError::OtherError("Data directory does not exist".to_string()));
        }
        Ok(())
    }

    pub async fn check_existing_state(&self) -> Result<DBState, SetupError> {
        // check if the folder exists or not
        if self.check_data_directory(DATA_DIR).is_err() {
            return Ok(DBState::NotReady);
        }

        println!("🗄️ Checking existing databases...");

        let data_dir = REPO_ROOT.join(DATA_DIR);
        let status_file = data_dir.join("STATUS");

        println!("{:?} status_filestatus_filestatus_filestatus_file ", status_file);

        let status = match fs::read_to_string(&status_file).await {
            Ok(s) => s.into(),
            Err(_) => DBState::NotReady,
        };

        println!("🔔 Pre Existing DB Status: {:?}", status);

        if status == DBState::ReadyToUse {
            self.validate_required_files(&data_dir)?;
        }

        Ok(status)
    }

    pub async fn copy_for_test(&self, dir_name: &str) -> Result<(), SetupError> {
        println!("🧑‍💻 Copying databases to {}", dir_name);

        let mut options = CopyOptions::new();
        options.overwrite = true;
        options.copy_inside = true;

        let data_directory = get_file_path(DATA_DIR);
        let data_test_directory = get_file_path(&dir_name);

        copy(&data_directory, &data_test_directory, &options).map_err(|e| SetupError::OtherError(e.to_string()))?;

        Ok(())
    }

    pub async fn mark_as_ready(&self) -> Result<(), SetupError> {
        let data_dir = get_file_path(DATA_DIR);
        let status_file = data_dir.join("STATUS");

        // Create the directory if it doesn't exist
        fs::create_dir_all(&data_dir)
            .await
            .map_err(|e| SetupError::OtherError(format!("Failed to create data directory: {}", e)))?;

        // Write the status file (this will create the file if it doesn't exist)
        fs::write(&status_file, "READY")
            .await
            .map_err(|e| SetupError::OtherError(format!("Failed to write status file: {}", e)))?;

        Ok(())
    }

    fn validate_required_files(&self, data_dir: &PathBuf) -> Result<(), SetupError> {
        let anvil_json_exists = data_dir.join(ANVIL_DATABASE_FILE).exists();
        let madara_db_exists = data_dir.join(MADARA_DATABASE_DIR).exists();
        let mock_verifier_exists = data_dir.join(MOCK_VERIFIER_ADDRESS_FILE).exists();
        let orchestrator_dir_exists = data_dir.join(ORCHESTRATOR_DATABASE_NAME).exists();


        if anvil_json_exists && madara_db_exists && address_json_exists && mock_verifier_exists {
            Ok(())
        } else {
            Err(SetupError::OtherError(
                "Database files missing despite ReadyToUse status".to_string()
            ))
        }
        Ok(())
    }
}
