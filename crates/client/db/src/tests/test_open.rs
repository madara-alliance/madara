use super::common::*;
use crate::DatabaseService;
use mc_metrics::MetricsRegistry;
use mp_chain_config::ChainConfig;
use mp_utils::tests_common::*;
use rstest::*;

#[rstest]
#[tokio::test]
async fn test_open_db(_set_workdir: ()) {
    temp_db::temp_db().await;
}

#[rstest]
#[tokio::test]
async fn test_open_different_chain_id(_set_workdir: ()) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    {
        let chain_config = std::sync::Arc::new(
            ChainConfig::starknet_integration().expect("failed to retrieve integration chain config"),
        );
        let _db =
            DatabaseService::new(temp_dir.path(), None, false, chain_config, &MetricsRegistry::dummy()).await.unwrap();
    }
    let chain_config = std::sync::Arc::new(ChainConfig::test_config().expect("failed to retrieve test chain config"));
    assert!(DatabaseService::new(temp_dir.path(), None, false, chain_config, &MetricsRegistry::dummy()).await.is_err());
}
