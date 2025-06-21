#[cfg(test)]
use {
    super::common::*,
    crate::{DatabaseService, MadaraBackendConfig},
    mp_chain_config::ChainConfig,
};

#[tokio::test]
async fn test_open_db() {
    temp_db::temp_db().await;
}

#[tokio::test]
async fn test_open_different_chain_id() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(ChainConfig::starknet_integration());
        let _db = DatabaseService::new(chain_config, MadaraBackendConfig::new(&temp_dir)).await.unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::madara_test());
    assert!(DatabaseService::new(chain_config, MadaraBackendConfig::new(&temp_dir)).await.is_err());
}
