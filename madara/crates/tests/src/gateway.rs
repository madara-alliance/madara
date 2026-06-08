#[cfg(test)]
mod tests {
    use crate::{
        devnet::{ACCOUNT_ADDRESS, ACCOUNT_SECRET, ERC20_STRK_CONTRACT_ADDRESS},
        wait_for_cond, MadaraCmd, MadaraCmdBuilder,
    };
    use anyhow::ensure;
    use mc_gateway_client::{BlockId, GatewayProvider};
    use mp_convert::ToFelt;
    use mp_state_update::StateDiff;
    use rstest::{fixture, rstest};
    use starknet::{
        accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
        signers::{LocalWallet, SigningKey},
    };
    use starknet_api::core::ChainId;
    use starknet_core::{
        types::{BlockId as RpcBlockId, BlockTag, Call, Felt},
        utils::starknet_keccak,
    };
    use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
    use std::time::Duration;
    use url::Url;

    #[fixture]
    fn client_mainnet_fixture() -> GatewayProvider {
        GatewayProvider::starknet_alpha_mainnet()
    }

    #[fixture]
    fn client_sepolia_fixture() -> GatewayProvider {
        GatewayProvider::starknet_alpha_sepolia()
    }

    fn local_gateway_client(node: &MadaraCmd) -> GatewayProvider {
        GatewayProvider::new(Url::parse(&node.gateway_url()).unwrap(), Url::parse(&node.feeder_gateway_url()).unwrap())
    }

    async fn devnet_account(
        node: &MadaraCmd,
        block_id: RpcBlockId,
    ) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
        let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
        let mut account = SingleOwnerAccount::new(
            node.json_rpc(),
            signer,
            ACCOUNT_ADDRESS,
            node.json_rpc().chain_id().await.unwrap(),
            ExecutionEncoding::New,
        );
        account.set_block_id(block_id);
        account
    }

    fn transfer_call() -> Vec<Call> {
        vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 1u64.into(), Felt::ZERO],
        }]
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

    #[rstest]
    #[tokio::test]
    async fn feeder_gateway_transaction_endpoints_local_devnet() {
        let args = [
            "--devnet",
            "--no-l1-sync",
            "--l1-gas-price",
            "1",
            "--blob-gas-price",
            "1",
            "--chain-config-override",
            "block_time=1s",
            "--gateway",
        ];

        let mut node = MadaraCmdBuilder::new().label("gateway").enable_gateway().args(args).run();
        node.wait_for_ready().await;

        let account = devnet_account(&node, RpcBlockId::Tag(BlockTag::Latest)).await;
        let tx_hash = account.execute_v3(transfer_call()).send().await.unwrap().transaction_hash;

        let receipt = wait_for_cond(
            || async {
                let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
                ensure!(receipt.block.is_block(), "tx not confirmed yet");
                Ok(receipt)
            },
            Duration::from_millis(500),
            60,
        )
        .await;

        let block_hash = receipt.block.block_hash().expect("confirmed receipt should include block hash");
        let block_number = receipt.block.block_number();
        let client = local_gateway_client(&node);

        let status = client.get_transaction_status(tx_hash).await.unwrap();
        assert_eq!(serde_json::to_value(status.tx_status).unwrap(), serde_json::Value::String("ACCEPTED_ON_L2".into()));
        assert_eq!(
            serde_json::to_value(status.execution_status).unwrap(),
            serde_json::Value::String("SUCCEEDED".into())
        );
        assert_eq!(status.block_hash, Some(block_hash));

        let transaction = client.get_transaction(tx_hash).await.unwrap();
        assert_eq!(
            serde_json::to_value(transaction.status).unwrap(),
            serde_json::Value::String("ACCEPTED_ON_L2".into())
        );
        assert_eq!(transaction.block_hash, Some(block_hash));
        assert_eq!(transaction.block_number, Some(block_number));
        assert!(transaction.transaction_index.is_some());
        assert_eq!(transaction.transaction.as_ref().unwrap().transaction_hash(), &tx_hash);

        assert_eq!(client.get_block_hash_by_id(block_number).await.unwrap(), block_hash);
        assert_eq!(client.get_block_id_by_hash(block_hash).await.unwrap(), block_number);
    }
}
