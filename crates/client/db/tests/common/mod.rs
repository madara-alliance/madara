use mc_db::DatabaseService;
use mp_chain_config::ChainConfig;
use tempfile::TempDir;

pub async fn temp_db() -> DatabaseService {
    let temp_dir = TempDir::new().unwrap();
    let chain_config = std::sync::Arc::new(ChainConfig::test_config().expect("failed to retrieve test chain config"));
    DatabaseService::new(temp_dir.path(), None, false, chain_config).await.unwrap()
}
