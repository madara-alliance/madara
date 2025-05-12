use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::worker::event_handler::triggers::batching::BatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::*;
use url::Url;

#[rstest]
#[tokio::test]
async fn test_assign_batch_to_block_new_batch() -> color_eyre::Result<()> {
    // Create test config
    let services = TestConfigBuilder::new()
        .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(Url::parse("https://pathfinder-madara-ci.karnot.xyz")?)))
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .build()
        .await;

    // Run the test
    let trigger = BatchingTrigger;
    let result = trigger.run_worker(services.config).await;
    assert!(result.is_ok());
    Ok(())
}
