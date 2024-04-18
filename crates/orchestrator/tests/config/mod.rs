use crate::common::default_job_item;

use super::common::{
    get_or_init_config,
    constants::{
        DA_LAYER, MADARA_RPC_URL, MONGODB_CONNECTION_STRING,
    }
};

use orchestrator::{
    config::Config,
    jobs::types::JobItem,
};

use rstest::*;
use starknet::providers::Provider;

use starknet::core::types::FieldElement;


#[fixture]
fn vec_of_field_elements() -> Vec<FieldElement> {
    vec![FieldElement::ONE, FieldElement::TWO, FieldElement::THREE]
}

#[rstest(get_or_init_config(String::from("http://localhost:9944")))]
#[tokio::test]
async fn test_valid_config(
    #[future] get_or_init_config: &'static dyn Config,
) {
    let config = get_or_init_config.await;
    config.starknet_client();
    config.da_client();
    config.database();
    config.queue();
}

#[rstest]
#[case::pass(
    String::from(MADARA_RPC_URL),
    String::from(MONGODB_CONNECTION_STRING),
    String::from(DA_LAYER),
)]
#[case::pass(
    String::from("http://invalid::invalid"), // No new config gets created
    String::from(MONGODB_CONNECTION_STRING),
    String::from(DA_LAYER),
)]
#[tokio::test]
async fn test_init_config_runs_ony_once(
    #[case] rpc_url: String, #[case] db_url: String, #[case] da_url: String 
) {
    let config = get_or_init_config(rpc_url, db_url, da_url).await;
    config.starknet_client();
    config.da_client();
    config.database();
    config.queue();
}

#[ignore = "Run this separately as it will fail the other tests (config can't be created with each test)"]
#[rstest(get_or_init_config(String::from("http://invalid:invalid")))]
#[should_panic]
#[tokio::test]
async fn test_invalid_config(
    #[future] get_or_init_config: &'static dyn Config,
) {
    get_or_init_config.await;
}

#[rstest]
#[tokio::test]
async fn test_config_starknet_client(
    #[future] get_or_init_config: &'static dyn Config,
) {
    let config = get_or_init_config.await;
    let result = config.starknet_client().block_number().await;
    assert!(result.is_ok(), "Failed to run starknet_client()");
}

#[rstest]
#[should_panic]
#[tokio::test]
async fn test_config_da_client(
    #[future] get_or_init_config: &'static dyn Config,
    vec_of_field_elements: Vec<FieldElement>,
) {
    let config = get_or_init_config.await;
    
    let result = config.da_client().publish_state_diff(vec_of_field_elements).await;
    assert!(result.is_err());
}

#[rstest]
#[tokio::test]
async fn test_config_database(
    #[future] get_or_init_config: &'static dyn Config,
    default_job_item: JobItem,
) {
    let config = get_or_init_config.await;
    let job = default_job_item;
    let result = config.database().create_job(job).await;
    assert!(result.is_err());
}