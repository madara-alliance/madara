#[cfg(test)]
mod tests {
    use mc_gateway_client::{BlockId, GatewayProvider};
    use mp_convert::ToFelt;
    use mp_state_update::StateDiff;
    use rstest::{fixture, rstest};
    use starknet_api::core::ChainId;
    use url::Url;

    #[fixture]
    fn client_mainnet_fixture() -> GatewayProvider {
        GatewayProvider::starknet_alpha_mainnet()
    }

    #[fixture]
    fn client_sepolia_fixture() -> GatewayProvider {
        GatewayProvider::starknet_alpha_sepolia()
    }
    /// Paradex testnet provider using config from configs/presets/paradex_testnet.yaml
    #[fixture]
    fn client_paradex_testnet_fixture() -> GatewayProvider {
        GatewayProvider::new(
            Url::parse("https://potc-testnet-sepolia.starknet.io/gateway/")
                .expect("Failed to parse Paradex testnet gateway url"),
            Url::parse("https://feeder.potc-testnet-sepolia.starknet.io/feeder_gateway/")
                .expect("Failed to parse Paradex testnet feeder gateway url"),
        )
    }

    #[rstest]
    #[case::v0_13_0(501_514)]
    #[case::v0_13_1(607_878)]
    #[case::v0_13_1_1(632_915)]
    #[case::v0_13_2(671_813)]
    #[case::v0_13_2_1(690_920)]
    #[case::v0_13_3(934_457)]
    #[case::v0_13_4(1_256_350)]
    #[case::v0_13_5(1_258_780)]
    #[case::v0_13_6(1_556_533)]
    #[tokio::test]
    async fn get_block_compute_hash_header(client_mainnet_fixture: GatewayProvider, #[case] block_n: u64) {
        let res = client_mainnet_fixture.get_state_update_with_block(BlockId::Number(block_n)).await.unwrap();
        println!("expected_block_hash: 0x{:x}", res.block.block_hash);
        let chain_id = ChainId::Mainnet.to_felt();
        let computed_block_hash =
            res.block.header(&res.state_update.state_diff.into()).unwrap().compute_hash(chain_id, false);
        println!("computed_block_hash: 0x{:x}", computed_block_hash);
        assert!(computed_block_hash == res.block.block_hash, "Computed block hash does not match expected block hash");
    }

    /// Sepolia v0.14.1 block with migrated_compiled_classes (SNIP-34)
    #[rstest]
    #[case::v0_14_1_snip34(2_934_726)] // First block with 7 migrated classes
    #[tokio::test]
    async fn get_block_compute_hash_header_sepolia(client_sepolia_fixture: GatewayProvider, #[case] block_n: u64) {
        let res = client_sepolia_fixture.get_state_update_with_block(BlockId::Number(block_n)).await.unwrap();
        let chain_id = ChainId::Sepolia.to_felt();
        let state_diff: StateDiff = res.state_update.state_diff.into();
        let computed = res.block.header(&state_diff).unwrap().compute_hash(chain_id, false);
        assert_eq!(computed, res.block.block_hash);
    }

    /// Paradex testnet block hash computation with detailed logging
    /// Chain ID: PRIVATE_SN_POTC_SEPOLIA
    #[rstest]
    #[case::block_481007(481007)]
    #[tokio::test]
    async fn get_block_compute_hash_header_paradex_testnet(
        client_paradex_testnet_fixture: GatewayProvider,
        #[case] block_n: u64,
    ) {
        use starknet_types_core::felt::Felt;

        let res = client_paradex_testnet_fixture.get_state_update_with_block(BlockId::Number(block_n)).await.unwrap();

        // Paradex testnet chain ID from config
        let chain_id = Felt::from_bytes_be_slice(b"PRIVATE_SN_POTC_SEPOLIA");

        let state_diff: StateDiff = res.state_update.state_diff.into();
        let header = res.block.header(&state_diff).unwrap();
        let computed_block_hash = header.compute_hash(chain_id, false);

        assert_eq!(computed_block_hash, res.block.block_hash, "Computed block hash does not match expected block hash");
    }
}
