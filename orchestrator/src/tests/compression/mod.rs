use crate::compression::blob::{convert_felt_vec_to_blob_data, da_word, state_update_to_blob_data};
use crate::compression::squash::squash_state_updates;
use crate::compression::stateful::compress as stateful_compress;
use crate::compression::stateless::compress as stateless_compress;
use crate::core::config::StarknetVersion;
use crate::tests::jobs::batching_job::convert_biguints_to_felts;
use crate::tests::utils::{
    build_test_config_with_real_provider, read_biguint_from_file, read_file_to_string, read_state_update_from_file,
    read_state_updates_vec_from_file,
};
use crate::worker::utils::biguint_vec_to_u8_vec;
use color_eyre::eyre::Result;
use rstest::*;
use starknet_core::types::Felt;

#[rstest]
#[tokio::test]
async fn test_squash_state_updates() -> Result<()> {
    let services = build_test_config_with_real_provider().await?;

    let state_updates_vector = read_state_updates_vec_from_file(&format!(
        "{}/src/tests/artifacts/8373665/state_updates.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_squashed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/8373665/squashed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let squashed_state_update =
        squash_state_updates(state_updates_vector, Some(789877), services.config.madara_client()).await?;

    assert_eq!(squashed_state_update, expected_squashed_state_update);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_stateful_compression() -> Result<()> {
    let services = build_test_config_with_real_provider().await?;

    let uncompressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/8373665/squashed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_compressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/8373665/stateful_compressed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let compressed_state_update =
        stateful_compress(&uncompressed_state_update, 789877, services.config.madara_client()).await?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    Ok(())
}

#[rstest]
#[case(StarknetVersion::V0_13_5)]
#[tokio::test]
async fn test_stateless_compression(#[case] version: StarknetVersion) -> Result<()> {
    let uncompressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/8373665/stateful_compressed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_compressed_state_update = convert_biguints_to_felts(&read_biguint_from_file(&format!(
        "{}/src/tests/artifacts/8373665/stateless_compressed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?)?;

    // Get a vector of felts from the compressed state update
    let vec_felts = state_update_to_blob_data(uncompressed_state_update, version).await?;

    // Perform stateless compression
    let compressed_state_update = stateless_compress(&vec_felts)?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_felt_vec_to_blob_data() -> Result<()> {
    let vec_felts = convert_biguints_to_felts(&read_biguint_from_file(&format!(
        "{}/src/tests/artifacts/8373665/stateless_compressed_state_update.json",
        env!("CARGO_MANIFEST_DIR")
    ))?)?;

    let blobs = convert_felt_vec_to_blob_data(&vec_felts)?;
    for (index, blob) in blobs.iter().enumerate() {
        assert_eq!(
            hex::encode(biguint_vec_to_u8_vec(blob.as_slice())),
            read_file_to_string(&format!(
                "{}/src/tests/artifacts/8373665/blobs/{}.txt",
                env!("CARGO_MANIFEST_DIR"),
                index + 1
            ))?
        );
    }

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
