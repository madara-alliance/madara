use bytes::Bytes;
use rstest::rstest;
use serde_json::json;

use crate::tests::config::{ConfigType, TestConfigBuilder};

/// This test checks the ability to put and get data from AWS S3 using `AWSS3`.
/// It puts JSON data into a test bucket and retrieves it, verifying the data
/// matches what was originally uploaded.
/// Dependencies: `color_eyre`, `dotenvy`, `rstest`, `tokio`, `serde_json`.
#[rstest]
#[tokio::test]
async fn test_put_and_get_data_s3() -> color_eyre::Result<()> {
    let services = TestConfigBuilder::new().configure_storage_client(ConfigType::Actual).build().await;

    let s3_client = services.config.storage();

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
