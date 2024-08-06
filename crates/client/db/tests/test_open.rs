mod common;

use common::temp_db;
use dc_db::DatabaseService;
use dp_block::chain_config::ChainConfig;

#[tokio::test]
async fn test_open_db() {
    temp_db().await;
}

#[tokio::test]
async fn test_open_different_chain_id() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(ChainConfig::starknet_integration());
        let _db = DatabaseService::new(temp_dir.path(), None, false, chain_config).await.unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::test_config());
    assert!(DatabaseService::new(temp_dir.path(), None, false, chain_config).await.is_err());
}
