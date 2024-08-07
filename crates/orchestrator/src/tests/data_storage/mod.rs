use crate::data_storage::aws_s3::config::{AWSS3ConfigType, S3LocalStackConfig};
use crate::data_storage::aws_s3::AWSS3;
use crate::data_storage::{DataStorage, DataStorageConfig};
use crate::tests::config::TestConfigBuilder;
use bytes::Bytes;
use rstest::rstest;
use serde_json::json;
use utils::env_utils::get_env_var_or_panic;

/// This test checks the ability to put and get data from AWS S3 using `AWSS3`.
/// It puts JSON data into a test bucket and retrieves it, verifying the data
/// matches what was originally uploaded.
/// Dependencies: `color_eyre`, `dotenvy`, `rstest`, `tokio`, `serde_json`.
#[rstest]
#[tokio::test]
async fn test_put_and_get_data_s3() -> color_eyre::Result<()> {
    TestConfigBuilder::new().build().await;

    dotenvy::from_filename("../.env.test")?;

    let config = S3LocalStackConfig::new_from_env();
    let s3_client = AWSS3::new(AWSS3ConfigType::WithEndpoint(config)).await;
    s3_client.build_test_bucket(&get_env_var_or_panic("AWS_S3_BUCKET_NAME")).await.unwrap();

    let mock_data = json!(
        {
            "body" : "hello world. hello world."
        }
    );
    let json_bytes = serde_json::to_vec(&mock_data)?;
    let key = "test_data.txt";

    // putting test data on key : "test_data.txt"
    s3_client.put_data(Bytes::from(json_bytes), key).await.expect("Unable to put data into the bucket.");

    // getting the data from key : "test_data.txt"
    let data = s3_client.get_data(key).await.expect("Unable to get the data from the bucket.");
    let received_json: serde_json::Value = serde_json::from_slice(&data)?;

    assert_eq!(received_json, mock_data);

    Ok(())
}
