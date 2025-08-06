use crate::compression::blob::{convert_felt_vec_to_blob_data, da_word, state_update_to_blob_data};
use crate::compression::squash::squash;
use crate::compression::stateful::compress as stateful_compress;
use crate::compression::stateless::compress as stateless_compress;
use crate::core::config::StarknetVersion;
use crate::tests::config::TestConfigBuilderReturns;
use crate::tests::utils::{
    build_test_config_with_real_provider, read_felt_vec_from_file, read_file_to_string, read_state_update_from_file,
    read_state_updates_vec_from_file,
};
use crate::worker::utils::biguint_vec_to_u8_vec;
use color_eyre::eyre::Result;
use orchestrator_utils::test_utils::setup_test_data;
use rstest::*;
use starknet_core::types::Felt;
use tracing::info;

#[rstest]
#[case(StarknetVersion::V0_13_5)]
#[tokio::test]
async fn test_state_update_to_blob_data_flow(#[case] version: StarknetVersion) -> Result<()> {
    let services = build_test_config_with_real_provider().await?;

    info!("Running test_state_update_to_blob_data_flow");
    let data_dir = setup_test_data(vec![("8373665.tar.gz", true)]).await?;

    let state_updates_path = data_dir.path().join("8373665/state_updates.json").to_str().unwrap().to_string();
    let squashed_state_update_path =
        data_dir.path().join("8373665/squashed_state_update.json").to_str().unwrap().to_string();
    let comp_state_update_path =
        data_dir.path().join("8373665/stateful_compressed_state_update.json").to_str().unwrap().to_string();
    let stateless_comp_path =
        data_dir.path().join("8373665/stateless_compressed_state_update.json").to_str().unwrap().to_string();
    let blob_paths = vec![
        data_dir.path().join("8373665/blobs/1.txt").to_str().unwrap().to_string(),
        data_dir.path().join("8373665/blobs/2.txt").to_str().unwrap().to_string(),
    ];

    test_squash_state_updates(&services, &state_updates_path, &squashed_state_update_path).await?;
    test_stateful_compression(&services, &squashed_state_update_path, &comp_state_update_path).await?;
    test_stateless_compression(&comp_state_update_path, &stateless_comp_path, version).await?;
    test_felt_vec_to_blob_data(&stateless_comp_path, blob_paths).await?;

    Ok(())
}

async fn test_squash_state_updates(
    services: &TestConfigBuilderReturns,
    state_updates_path: &str,
    squashed_state_update_path: &str,
) -> Result<()> {
    let state_updates_vector = read_state_updates_vec_from_file(state_updates_path)?;
    let expected_squashed_state_update = read_state_update_from_file(squashed_state_update_path)?;

    let squashed_state_update =
        squash(state_updates_vector.iter().collect::<Vec<_>>(), Some(789877), services.config.madara_client()).await?;

    assert_eq!(squashed_state_update, expected_squashed_state_update);

    info!("[Tested] state updates to squash logic");

    Ok(())
}

async fn test_stateful_compression(
    services: &TestConfigBuilderReturns,
    squashed_state_update_path: &str,
    stateful_comp_path: &str,
) -> Result<()> {
    let uncompressed_state_update = read_state_update_from_file(squashed_state_update_path)?;
    let expected_compressed_state_update = read_state_update_from_file(stateful_comp_path)?;

    let compressed_state_update =
        stateful_compress(&uncompressed_state_update, 789877, services.config.madara_client()).await?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    info!("[Tested] squashed state update to stateful compressed state updated logic");

    Ok(())
}

async fn test_stateless_compression(
    stateful_comp_path: &str,
    stateless_comp_path: &str,
    version: StarknetVersion,
) -> Result<()> {
    let uncompressed_state_update = read_state_update_from_file(stateful_comp_path)?;
    let expected_compressed_state_update = read_felt_vec_from_file(stateless_comp_path)?;

    // Get a vector of felts from the compressed state update
    let vec_felts = state_update_to_blob_data(uncompressed_state_update, version).await?;

    // Perform stateless compression
    let compressed_state_update = stateless_compress(&vec_felts)?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    info!("[Tested] stateful compressed state update to stateless compressed state update logic");

    Ok(())
}

async fn test_felt_vec_to_blob_data(stateless_comp_path: &str, blob_paths: Vec<String>) -> Result<()> {
    let vec_felts = read_felt_vec_from_file(stateless_comp_path)?;

    let blobs = convert_felt_vec_to_blob_data(&vec_felts)?;
    for (index, blob) in blobs.iter().enumerate() {
        assert_eq!(hex::encode(biguint_vec_to_u8_vec(blob.as_slice())), read_file_to_string(&blob_paths[index])?);
    }

    info!("[Tested] stateless compressed state update to blob data logic");

    Ok(())
}

/// Tests `da_word` function with various inputs for class flag, new nonce, and number of
/// changes. Verifies that `da_word` produces the correct Felt based on the provided
/// parameters. Uses test cases with different combinations of inputs and expected output
/// strings. Asserts the function's correctness by comparing the computed and expected
/// Felts.
#[rstest]
#[case(false, 1, 1, "18446744073709551617", StarknetVersion::V0_13_2)]
#[case(false, 1, 0, "18446744073709551616", StarknetVersion::V0_13_2)]
#[case(false, 0, 6, "6", StarknetVersion::V0_13_2)]
#[case(true, 1, 0, "340282366920938463481821351505477763072", StarknetVersion::V0_13_2)]
#[case(false, 0, 15, "62", StarknetVersion::V0_13_5)]
#[case(false, 0, 1024, "4096", StarknetVersion::V0_13_5)]
#[case(false, 10, 0, "10242", StarknetVersion::V0_13_5)]
#[case(false, 15, 2025, "1106804644422573105060", StarknetVersion::V0_13_5)]
#[case(true, 0, 10, "43", StarknetVersion::V0_13_5)]
#[case(true, 0, 1000, "4001", StarknetVersion::V0_13_5)]
#[case(true, 20, 255, "21503", StarknetVersion::V0_13_5)]
#[case(true, 1000, 256, "73786976294838206465025", StarknetVersion::V0_13_5)]
#[tokio::test]
async fn test_da_word(
    #[case] class_flag: bool,
    #[case] new_nonce: u64,
    #[case] num_changes: u64,
    #[case] expected: String,
    #[case] version: StarknetVersion,
) {
    let new_nonce = if new_nonce > 0 { Some(Felt::from(new_nonce)) } else { None };
    let da_word = da_word(class_flag, new_nonce, num_changes, version).expect("Failed to create DA word");
    let expected = Felt::from_dec_str(expected.as_str()).unwrap();
    assert_eq!(da_word, expected);
}
