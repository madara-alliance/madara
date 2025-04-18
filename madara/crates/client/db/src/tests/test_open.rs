use super::common::*;
use crate::DatabaseService;
use mp_chain_config::ChainConfig;

#[tokio::test]
async fn test_open_db() {
    temp_db::temp_db().await;
}

#[tokio::test]
async fn test_open_different_chain_id() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(ChainConfig::starknet_integration());
        let _db = DatabaseService::new(temp_dir.path(), None, false, chain_config, Default::default(), Some(0)).await.unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::madara_test());
    assert!(DatabaseService::new(temp_dir.path(), None, false, chain_config, Default::default(), Some(0)).await.is_err());
}
