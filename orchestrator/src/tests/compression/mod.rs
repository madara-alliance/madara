use crate::compression::blob::state_update_to_blob_data;
use crate::compression::squash::squash_state_updates;
use crate::compression::stateful::compress as stateful_compress;
use crate::compression::stateless::compress as stateless_compress;
use crate::core::config::StarknetVersion;
use crate::tests::config::TestConfigBuilderReturns;
use crate::tests::utils::{
    build_test_config_with_real_provider, read_state_update_from_file, read_state_updates_vec_from_file,
};
use color_eyre::eyre::Result;
use rstest::*;
use std::io::Read;

#[rstest]
#[tokio::test]
async fn test_squash_state_updates(
    #[from(build_test_config_with_real_provider)] services: TestConfigBuilderReturns,
) -> Result<()> {
    let state_updates_vector = read_state_updates_vec_from_file(&format!(
        "{}/src/tests/artifacts/state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_squashed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/squashed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let squashed_state_update =
        squash_state_updates(state_updates_vector, Some(789877), services.config.madara_client()).await?;

    assert_eq!(squashed_state_update, expected_squashed_state_update);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_stateful_compression(
    #[from(build_test_config_with_real_provider)] services: TestConfigBuilderReturns,
) -> Result<()> {
    let uncompressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/squashed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_compressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/stateful_compressed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let compressed_state_update =
        stateful_compress(&uncompressed_state_update, 789877, services.config.madara_client()).await?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    Ok(())
}

#[rstest]
#[case("0.13.5")]
#[tokio::test]
async fn test_stateless_compression(#[case] version: &str) -> Result<()> {
    let uncompressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/stateful_compressed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    let expected_compressed_state_update = read_state_update_from_file(&format!(
        "{}/src/tests/artifacts/stateless_compressed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    // Get a vector of felts from the compressed state update
    // TODO: use the version sent using the case above
    let vec_felts = state_update_to_blob_data(uncompressed_state_update, StarknetVersion::V0_13_5).await?;

    // Perform stateless compression
    let compressed_state_update = stateless_compress(&vec_felts).await?;

    assert_eq!(compressed_state_update, expected_compressed_state_update);

    Ok(())
}
