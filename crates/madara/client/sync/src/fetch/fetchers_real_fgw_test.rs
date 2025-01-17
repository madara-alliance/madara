//! These tests use the real FGW. They are very basic compared to the mock tests.

use super::*;
use rstest::{fixture, rstest};

#[fixture]
fn client_mainnet_fixture() -> GatewayProvider {
    GatewayProvider::starknet_alpha_mainnet()
}

#[rstest]
#[tokio::test]
async fn test_can_fetch_pending_block(client_mainnet_fixture: GatewayProvider) {
    let block = fetch_pending_block_and_updates(Felt::ZERO, &ChainId::Mainnet, &client_mainnet_fixture).await.unwrap();
    // ignore as we can't check much here :/
    drop(block);
}

// TODO: I'd like to have more tests for more starknt versions, but i don't want to commit multiple megabytes into the repository. These tests should be made with the mocked FGW.
#[tokio::test]
#[rstest]
#[case(0)]
#[case(724_130)]
async fn test_can_fetch_and_convert_block(client_mainnet_fixture: GatewayProvider, #[case] block_n: u64) {
    // Sorting is necessary since we store storage diffs and nonces in a
    // hashmap in the fgw types before converting them to a Vec in the mp
    // types, resulting in unpredictable ordering
    let mut block = fetch_block_and_updates(&ChainId::Mainnet, block_n, &client_mainnet_fixture).await.unwrap();
    block.state_diff.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
    block.state_diff.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));

    let path = &format!("test-data/block_{block_n}.json");
    // serde_json::to_writer(std::fs::File::create(path).unwrap(), &block).unwrap();
    let mut expected: UnverifiedFullBlock = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    expected.state_diff.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
    expected.state_diff.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));

    // let path = &format!("test-data/block_{block_n}_actual.json");
    // serde_json::to_writer(std::fs::File::create(path).unwrap(), &block).unwrap();

    assert_eq!(block, expected)
}
