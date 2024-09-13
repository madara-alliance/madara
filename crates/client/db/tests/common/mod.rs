use std::{env, path::PathBuf};

use mc_db::DatabaseService;
use mp_chain_config::ChainConfig;
use rstest::fixture;
use tempfile::TempDir;

#[fixture]
pub fn set_workdir() {
    let output = std::process::Command::new("cargo")
        .arg("locate-project")
        .arg("--workspace")
        .arg("--message-format=plain")
        .output()
        .expect("Failed to execute command");

    let cargo_toml_path = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let project_root = PathBuf::from(cargo_toml_path.trim()).parent().unwrap().to_path_buf();
    
    env::set_current_dir(&project_root).expect("Failed to set working directory");
}

pub async fn temp_db() -> DatabaseService {
    let temp_dir = TempDir::new().unwrap();
    let chain_config = std::sync::Arc::new(ChainConfig::test_config().expect("failed to retrieve test chain config"));
    DatabaseService::new(temp_dir.path(), None, false, chain_config).await.unwrap()
}
