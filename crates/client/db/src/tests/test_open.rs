use super::common::*;
use crate::DatabaseService;
use mc_metrics::MetricsRegistry;
use mp_chain_config::ChainConfig;
use rstest::*;

#[rstest]
#[tokio::test]
async fn test_open_db() {
    temp_db::temp_db().await;
}

#[rstest]
#[tokio::test]
async fn test_open_different_chain_id() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(ChainConfig::starknet_integration());
        let _db =
            DatabaseService::new(temp_dir.path(), None, false, chain_config, &MetricsRegistry::dummy()).await.unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::madara_devnet());
    assert!(DatabaseService::new(temp_dir.path(), None, false, chain_config, &MetricsRegistry::dummy()).await.is_err());
}
