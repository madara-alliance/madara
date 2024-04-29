use std::path::PathBuf;

use sc_service::Configuration;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Configs {
    pub remote_base_path: String,
    pub genesis_assets: Vec<FileInfos>,
}

#[derive(Deserialize)]
pub struct FileInfos {
    pub name: String,
    pub sha3_256: Option<String>,
    pub url: Option<String>,
}

/// Returns the path to the database of the node.
pub fn db_config_dir(config: &Configuration) -> PathBuf {
    config.base_path.config_dir(config.chain_spec.id())
}
