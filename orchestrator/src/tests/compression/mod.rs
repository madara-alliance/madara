use crate::compression::squash::squash_state_updates;
use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;
use color_eyre::eyre::Result;
use rstest::*;
use starknet_core::types::StateUpdate;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use tracing::log::warn;
use url::Url;

#[rstest]
#[tokio::test]
async fn test_squash_state_updates() -> Result<()> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            warn!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return Ok(());
        }
    };

    let services =
        TestConfigBuilder::new().configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url))).build().await;

    // Read the input file content
    let mut state_updates_string = String::new();
    let mut file = File::open(Path::new(&format!(
        "{}/src/tests/artifacts/state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    )))?;
    file.read_to_string(&mut state_updates_string).expect("Failed to read state update");

    // Parse as Vec<StateUpdate>
    let state_updates_vector: Vec<StateUpdate> = serde_json::from_str(&state_updates_string)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update file: {}", e))?;

    // Read the output file content
    let mut expected_squashed_state_update_string = String::new();
    let mut file = File::open(Path::new(&format!(
        "{}/src/tests/artifacts/squashed_state_update_789878_789900.json",
        env!("CARGO_MANIFEST_DIR")
    )))?;
    file.read_to_string(&mut expected_squashed_state_update_string).expect("Failed to read state update");

    // Parse as StateUpdate
    let expected_squashed_state_update: StateUpdate = serde_json::from_str(&expected_squashed_state_update_string)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse state update file: {}", e))?;

    let squashed_state_update =
        squash_state_updates(state_updates_vector, Some(789877), services.config.madara_client()).await?;

    assert_eq!(squashed_state_update, expected_squashed_state_update);

    Ok(())
}
