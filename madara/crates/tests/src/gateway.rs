#[cfg(test)]
mod tests {
    use mc_gateway_client::GatewayProvider;
    use mp_block::BlockId;
    use mp_convert::ToFelt;
    use rstest::{fixture, rstest};
    use starknet_api::core::ChainId;

    #[fixture]
    fn client_mainnet_fixture() -> GatewayProvider {
        GatewayProvider::starknet_alpha_mainnet()
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
}
