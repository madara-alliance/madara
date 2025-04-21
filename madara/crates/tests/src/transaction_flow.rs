use crate::{
    devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ERC20_STRK_CONTRACT_ADDRESS, SEQUENCER_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use assert_matches::assert_matches;
use rstest::rstest;
use starknet::accounts::{Account, AccountError, ConnectedAccount};
use starknet::{
    accounts::{ExecutionEncoding, SingleOwnerAccount},
    signers::{LocalWallet, SigningKey},
};
use starknet_core::{
    types::{
        BlockId, BlockTag, Call, ExecutionResult, Felt, FunctionCall, StarknetError, TransactionReceipt,
        TransactionReceiptWithBlockInfo,
    },
    utils::starknet_keccak,
};
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError, SequencerGatewayProvider};
use std::time::Duration;

enum TestSetup {
    /// Sequencer only, offering a jsonrpc and gateway interface. Transactions are validated and executed on the same node.
    SequencerOnly,

    /// 2 nodes:
    /// - Full node syncs from the sequencer gateway interface. This node offers jsonrpc and gateway interface,
    ///   and will not perform transaction validation.
    /// - Sequencer node performs transaction validation and execution.
    FullNodeAndSequencer,

    /// 2 nodes:
    /// - Gateway node syncs from the sequencer gateway interface. This node offers jsonrpc and gateway interface.
    ///   Transaction validation is performed on the gateway node, and validated transactions are forwarded to the sequencer using
    ///   a madara-specific gateway endpoint.
    /// - Sequencer node handles execution.
    ///
    /// The main difference between `FullNodeAndSequencer` and `GatewayAndSequencer` is where transaction validation is
    /// performed - both offer the same user-facing APIs, but the latter requires the use of the madara-specific add_validated_tx
    /// gateway endpoint.
    GatewayAndSequencer,
}

use TestSetup::*;

impl TestSetup {
    pub async fn run(self) -> RunningTestSetup {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        match self {
            SequencerOnly => Self::run_single_node().await,
            FullNodeAndSequencer => Self::run_full_node_and_sequencer().await,
            GatewayAndSequencer => Self::run_gateway_and_sequencer().await,
        }
    }

    async fn run_single_node() -> RunningTestSetup {
        // sequencer
        let sequencer = MadaraCmdBuilder::new().label("sequencer").enable_gateway().args([
            "--devnet",
            "--no-l1-sync",
            "--gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            "block_time=500min,pending_block_update_time=500ms",
            "--gateway",
        ]);

        let mut sequencer = sequencer.run();
        sequencer.wait_for_sync_to(0).await;
        RunningTestSetup::SingleNode(sequencer)
    }

    async fn run_gateway_and_sequencer() -> RunningTestSetup {
        // sequencer
        let sequencer = MadaraCmdBuilder::new().label("sequencer").enable_gateway().args([
            "--devnet",
            "--no-l1-sync",
            "--gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            "block_time=500min,pending_block_update_time=500ms",
            "--gateway",
            "--gateway-trusted-add-transaction-endpoint",
        ]);
        // validator
        let gateway = MadaraCmdBuilder::new().label("gateway").enable_gateway().args([
            "--full",
            "--no-l1-sync",
            "--gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            &format!(
                "gateway_url=\"{}\",feeder_gateway_url=\"{}\"",
                sequencer.gateway_url(),
                sequencer.feeder_gateway_url()
            ),
            "--validate-then-forward-txs-to",
            &format!("{}/madara", sequencer.gateway_root_url()),
            "--gateway",
        ]);

        let mut sequencer = sequencer.run();
        sequencer.wait_for_sync_to(0).await;
        let mut gateway = gateway.run();
        gateway.wait_for_sync_to(0).await;
        RunningTestSetup::TwoNodes { _sequencer: sequencer, user_facing: gateway }
    }

    async fn run_full_node_and_sequencer() -> RunningTestSetup {
        // sequencer
        let sequencer = MadaraCmdBuilder::new().label("sequencer").enable_gateway().args([
            "--devnet",
            "--no-l1-sync",
            "--gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            "block_time=500min,pending_block_update_time=500ms",
            "--gateway",
        ]);
        // validator
        let full_node = MadaraCmdBuilder::new().label("full_node").enable_gateway().args([
            "--full",
            "--no-l1-sync",
            "--gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            &format!(
                "gateway_url=\"{}\",feeder_gateway_url=\"{}\"",
                sequencer.gateway_url(),
                sequencer.feeder_gateway_url()
            ),
            "--gateway",
        ]);

        let mut sequencer = sequencer.run();
        sequencer.wait_for_sync_to(0).await;
        let mut gateway = full_node.run();
        gateway.wait_for_sync_to(0).await;
        RunningTestSetup::TwoNodes { _sequencer: sequencer, user_facing: gateway }
    }
}

enum RunningTestSetup {
    SingleNode(MadaraCmd),
    TwoNodes { _sequencer: MadaraCmd, user_facing: MadaraCmd },
}

impl RunningTestSetup {
    pub fn user_facing_node(&self) -> &MadaraCmd {
        match self {
            RunningTestSetup::SingleNode(sequencer) => sequencer,
            RunningTestSetup::TwoNodes { user_facing, .. } => user_facing,
        }
    }

    pub async fn chain_id(&self) -> Felt {
        self.json_rpc().chain_id().await.unwrap()
    }

    pub fn json_rpc(&self) -> &JsonRpcClient<HttpTransport> {
        self.user_facing_node().json_rpc()
    }

    pub async fn gateway_client(&self) -> SequencerGatewayProvider {
        self.user_facing_node().gateway_client(self.chain_id().await)
    }

    pub async fn account<P: Provider + Sync + Send>(&self, provider: P) -> SingleOwnerAccount<P, LocalWallet> {
        let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
        let mut account =
            SingleOwnerAccount::new(provider, signer, ACCOUNT_ADDRESS, self.chain_id().await, ExecutionEncoding::New);
        account.set_block_id(BlockId::Tag(BlockTag::Pending));
        account
    }

    pub async fn expect_tx_receipt(&self, tx_hash: Felt) -> TransactionReceiptWithBlockInfo {
        wait_for_cond(
            || async { Ok(self.json_rpc().get_transaction_receipt(tx_hash).await?) },
            Duration::from_millis(500),
            60,
        )
        .await
    }

    pub async fn get_balance(&self, account: Felt) -> Vec<Felt> {
        // can only be called via jsonrpc
        self.json_rpc()
            .call(
                &FunctionCall {
                    contract_address: ERC20_STRK_CONTRACT_ADDRESS,
                    entry_point_selector: starknet_keccak(b"balance_of"),
                    calldata: vec![account],
                },
                BlockId::Tag(BlockTag::Pending),
            )
            .await
            .unwrap()
    }

    pub async fn get_nonce(&self, contract_address: Felt) -> Felt {
        // can only be called via jsonrpc
        self.json_rpc().get_nonce(BlockId::Tag(BlockTag::Pending), contract_address).await.unwrap()
    }
}

fn make_transfer_call(recipient: Felt, amount: u128) -> Vec<Call> {
    vec![Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount.into(), Felt::ZERO],
    }]
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer)]
#[case::gateway(GatewayAndSequencer)]
#[case::single_node(SequencerOnly)]
async fn normal_transfer(#[case] setup: TestSetup) {
    let setup = setup.run().await;

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P) {
        let account = setup.account(provider).await;

        let recipient = ACCOUNTS[2];
        let amount = 15;

        let init_balance = setup.get_balance(recipient).await;
        let init_nonce = setup.get_nonce(account.address()).await;

        let res = account
            .execute_v3(make_transfer_call(recipient, amount))
            .gas_price(0x50) // we have to do this as gateway doesn't have the estimate_fee or get nonce method.
            .gas(0x100)
            .nonce(init_nonce)
            .send()
            .await
            .unwrap();

        let res = setup.expect_tx_receipt(res.transaction_hash).await;
        assert_eq!(res.receipt.execution_result(), &ExecutionResult::Succeeded);

        let TransactionReceipt::Invoke(receipt) = res.receipt else {
            unreachable!("tx receipt not invoke: {:?}", res.receipt)
        };

        let ev = receipt.events;
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].from_address, ERC20_STRK_CONTRACT_ADDRESS);
        assert_eq!(ev[0].keys, vec![starknet_keccak(b"Transfer"), ACCOUNTS[0], recipient]);
        assert_eq!(ev[0].data, vec![amount.into(), Felt::ZERO]);
        // Fee transfer to sequencer
        assert_eq!(ev[1].from_address, ERC20_STRK_CONTRACT_ADDRESS);
        assert_eq!(ev[1].keys, vec![starknet_keccak(b"Transfer"), ACCOUNTS[0], SEQUENCER_ADDRESS]);
        assert_eq!(ev[1].data.len(), 2);
        assert!(ev[1].data[0] > Felt::ZERO); // non-zero fee

        assert_eq!(setup.json_rpc().get_nonce(account.block_id(), account.address()).await.unwrap(), init_nonce + 1);
        assert_eq!(setup.get_balance(recipient).await, vec![init_balance[0] + Felt::from(amount), Felt::ZERO]);
    }

    // via gateway
    perform_test(&setup, &setup.gateway_client().await).await;

    // via rpc
    perform_test(&setup, setup.json_rpc()).await;
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer, false)]
// This is an interesting one: when we are not waiting for the initial transfer to be included in the chain,
// the gateway validation of the incorrect nonce transaction may succeed. In that case, the nonce error is actually
// caught on the sequencer node: it's a mempool insertion error instead of a gateway validation error. We also test this case :)
#[case::gateway_validation_error(GatewayAndSequencer, true)]
#[case::gateway_mempool_error(GatewayAndSequencer, false)]
#[case::single_node(SequencerOnly, false)]
async fn invalid_nonce(#[case] setup: TestSetup, #[case] wait_for_initial_transfer: bool) {
    let setup = setup.run().await;

    let init_nonce = setup.get_nonce(ACCOUNTS[0]).await;

    // do a tx to increment the account nonce
    let res = setup
        .account(setup.json_rpc())
        .await
        .execute_v3(make_transfer_call(ACCOUNTS[4], 1418283))
        .send()
        .await
        .unwrap();
    if wait_for_initial_transfer {
        setup.expect_tx_receipt(res.transaction_hash).await;
    }

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P, invalid_nonce: Felt) {
        let account = setup.account(provider).await;
        let res = account
            .execute_v3(make_transfer_call(ACCOUNTS[2], 1232))
            .gas_price(0x50)
            .gas(0x100)
            .nonce(invalid_nonce)
            .send()
            .await;
        assert_matches!(
            res.unwrap_err(),
            AccountError::Provider(ProviderError::StarknetError(StarknetError::InvalidTransactionNonce))
        );
    }

    // via gateway
    perform_test(&setup, &setup.gateway_client().await, init_nonce).await;

    // via rpc
    perform_test(&setup, setup.json_rpc(), init_nonce).await;
}
