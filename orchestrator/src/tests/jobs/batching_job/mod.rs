use crate::core::StorageClient;
use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;
use crate::tests::utils::read_file_to_string;
use crate::worker::event_handler::triggers::batching::BatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use alloy::hex;
use color_eyre::Result;
use num_bigint::BigUint;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_utils::test_utils::setup_test_data;
use rstest::*;
use starknet_core::types::Felt;
use tracing::warn;
use url::Url;

#[rstest]
#[case(789878, 790377)]
#[ignore = "Ignoring this test because it takes a long time on CI and we have individual tests for all the steps - squash, stateless, stateful, blob, etc."]
#[tokio::test]
async fn test_assign_batch_to_block_new_batch(
    #[case] min_block_to_process: u64,
    #[case] max_block_to_process: u64,
) -> Result<()> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            warn!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return Ok(());
        }
    };

    let mut mock_prover_client = MockProverClient::new();
    mock_prover_client.expect_submit_task().returning(|_| Ok("01234ABCD".to_string()));

    let services = TestConfigBuilder::new()
        .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url)))
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .configure_prover_client(mock_prover_client.into())
        .configure_min_block_to_process(min_block_to_process)
        .configure_max_block_to_process(Some(max_block_to_process))
        .build()
        .await;

    let result = BatchingTrigger.run_worker(services.config.clone()).await;

    assert!(result.is_ok());

    let generated_blobs =
        get_blobs_from_s3_paths(vec!["blob/batch/1/1.txt", "blob/batch/1/2.txt"], services.config.storage()).await?;

    // Fetch real blobs from test data
    // The test data contains state update information about block 8373665 on Ethereum Sepolia
    // Contains the following files:
    // blobs/1.txt
    // blobs/2.txt
    // ... other files
    let data_dir = setup_test_data(vec![("8373665.tar.gz", true)]).await?;
    let blob_dir = data_dir.path().join("8373665/blobs/").to_str().unwrap().to_string();

    let real_blobs = get_blobs_from_files(vec![&format!("{blob_dir}1.txt"), &format!("{blob_dir}2.txt")])?;

    assert_eq!(generated_blobs[0], real_blobs[0]);
    assert_eq!(generated_blobs[1], real_blobs[1]);

    Ok(())
}

async fn get_blobs_from_s3_paths(s3_paths: Vec<&str>, storage: &dyn StorageClient) -> Result<Vec<String>> {
    let mut blob: Vec<String> = Vec::new();
    for path in s3_paths {
        blob.push(hex::encode(storage.get_data(path).await?));
    }
    Ok(blob)
}

fn get_blobs_from_files(file_paths: Vec<&str>) -> Result<Vec<String>> {
    let mut blob: Vec<String> = Vec::new();
    for path in file_paths {
        blob.push(read_file_to_string(path)?);
    }
    Ok(blob)
}

/// Converts a vector of BigUint values to a vector of Felt values
///
/// # Arguments
/// * `biguints` - Vector of BigUint values to convert
///
/// # Returns
/// A Result containing a vector of Felt values or an error
pub fn convert_biguints_to_felts(biguints: &[BigUint]) -> Result<Vec<Felt>> {
    biguints
        .iter()
        .map(|b| {
            let bytes = b.to_bytes_be();
            // Handle empty bytes case
            if bytes.is_empty() {
                return Ok(Felt::ZERO);
            }

            // Create a fixed size array for the bytes
            let mut field_bytes = [0u8; 32];

            // Copy bytes, padding with zeros if needed
            if bytes.len() <= 32 {
                let start_idx = 32 - bytes.len();
                field_bytes[start_idx..].copy_from_slice(&bytes);
            } else {
                // Truncate if bigger than 32 bytes
                field_bytes.copy_from_slice(&bytes[bytes.len() - 32..]);
            }

            // Convert to Felt
            Ok(Felt::from_bytes_be(&field_bytes))
        })
        .collect()
}
