// =============================================================================
// DATABASE MANAGEMENT
// =============================================================================

use std::path::Path;
use tokio::fs;
use fs_extra::dir::{copy, CopyOptions};
// Import all the services we've created
use crate::services::mock_verifier::DEFAULT_VERIFIER_FILE_NAME;
use crate::services::bootstrapper::BOOTSTRAPPER_DEFAULT_ADDRESS_PATH;
use crate::services::anvil::ANVIL_DEFAULT_DATABASE_NAME;
use crate::services::madara::MADARA_DEFAULT_DATABASE_NAME;
pub use super::config::*;


pub struct DatabaseManager {
}

impl DatabaseManager {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn check_existing_state(&self) -> Result<DBState, SetupError> {
        println!("🗄️ Checking existing databases...");

        let data_dir = Path::new(DEFAULT_DATA_DIR);
        let status_file = data_dir.join("STATUS");

        let status = match fs::read_to_string(&status_file).await {
            Ok(s) => s.into(),
            Err(_) => DBState::NotReady,
        };

        println!("Pre Existing DB Status: {:?}", status);

        if status == DBState::ReadyToUse {
            self.validate_required_files(data_dir)?;
        }

        Ok(status)
    }

    pub async fn copy_for_test(&self, test_name: &str) -> Result<(), SetupError> {
        let data_test_name = format!("data_{}", test_name);
        println!("🧑‍💻 Copying databases to {}", data_test_name);

        let mut options = CopyOptions::new();
        options.overwrite = true;
        options.copy_inside = true;

        copy(DEFAULT_DATA_DIR, &data_test_name, &options)
            .map_err(|e| SetupError::OtherError(e.to_string()))?;

        Ok(())
    }

    pub async fn mark_as_ready(&self) -> Result<(), SetupError> {
        let data_dir = Path::new(DEFAULT_DATA_DIR);
        let status_file = data_dir.join("STATUS");

        fs::write(&status_file, "READY").await
            .map_err(|e| SetupError::OtherError(format!("Failed to write status file: {}", e)))?;

        Ok(())
    }

    fn validate_required_files(&self, data_dir: &Path) -> Result<(), SetupError> {
        let anvil_json_exists = data_dir.join(ANVIL_DEFAULT_DATABASE_NAME).exists();
        let madara_db_exists = data_dir.join(MADARA_DEFAULT_DATABASE_NAME).exists();
        let address_json_exists = data_dir.join(BOOTSTRAPPER_DEFAULT_ADDRESS_PATH).exists();
        let mock_verifier_exists = data_dir.join(DEFAULT_VERIFIER_FILE_NAME).exists();

        if anvil_json_exists && madara_db_exists && address_json_exists && mock_verifier_exists {
            Ok(())
        } else {
            Err(SetupError::OtherError(
                "Database files missing despite ReadyToUse status".to_string()
            ))
        }
    }
}
