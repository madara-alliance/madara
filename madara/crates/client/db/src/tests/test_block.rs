#[cfg(test)]
#[tokio::test]
async fn test_chain_info() {
    let db = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let chain_config = db.chain_config();
    assert_eq!(chain_config.chain_id, ChainConfig::madara_test().chain_id);
}
