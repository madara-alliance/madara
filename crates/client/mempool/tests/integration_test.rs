use std::sync::Arc;

use mc_db::MadaraBackend;
use mc_mempool::{block_production::BlockProductionTask, GasPriceProvider, L1DataProvider, Mempool};
use mp_chain_config::ChainConfig;

#[tokio::test]
async fn test_init_block_production_fails_no_genesis() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let l1_gas_setter = GasPriceProvider::new();
    let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());
    let mempool = Arc::new(Mempool::new(backend.clone(), l1_data_provider.clone()));
    let block_production_task = BlockProductionTask::new(backend, mempool, l1_data_provider);
    assert!(block_production_task.is_err());
    assert_eq!(block_production_task.err().unwrap().to_string(), "No genesis block in storage");
}

#[tokio::test]
async fn test_block_production_simple_flow() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = MadaraBackend::open_for_testing(chain_config.clone());
    let l1_gas_setter = GasPriceProvider::new();
    let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());
    let mempool = Arc::new(Mempool::new(backend.clone(), l1_data_provider.clone()));

    let mut block_production_task = BlockProductionTask::new(backend, mempool, l1_data_provider).expect("Failed to create block production");
    
    block_production_task.block_production_task().await.expect("Failed to run block production task");
}