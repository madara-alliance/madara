use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::tests::utils::read_blob_from_file;
use crate::worker::event_handler::triggers::batching::BatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use alloy::hex;
use rstest::*;
use url::Url;
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;

#[rstest]
#[case("src/tests/jobs/batching_job/test_data/blob/")]
#[tokio::test]
async fn test_assign_batch_to_block_new_batch(#[case] blob_dir: String) -> color_eyre::Result<()> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            println!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return Ok(());
        }
    };

    let services = TestConfigBuilder::new()
        .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url)))
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .build()
        .await;

    let result = BatchingTrigger.run_worker(services.config.clone()).await;

    assert!(result.is_ok());

    let blob = read_blob_from_file(format!("{blob_dir}6815891.txt")).expect("issue while reading blob from storage");
    let generated_blob = hex::encode(services.config.storage().get_data("blob/batch/1/1.txt").await?);

    assert_eq!(generated_blob, blob);

    Ok(())
}
