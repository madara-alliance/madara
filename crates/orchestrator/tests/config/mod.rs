use super::common::init_valid_config;

use rstest::*;
use starknet::providers::Provider;


#[rstest]
#[case::pass(String::from("http://localhost:9944"))]
#[case::fail(String::from("http://localhost:invalid"))]
#[tokio::test]
async fn test_valid_config(
    #[case] input_url: String,
) {
    let config = init_valid_config(input_url).await;

    let result = config.starknet_client().block_number().await;
    assert!(result.is_ok());
}