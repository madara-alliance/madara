use crate::{
    devnet::{
        ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ACCOUNT_SECRETS, ERC20_STRK_CONTRACT_ADDRESS, SEQUENCER_ADDRESS,
    },
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::bail;
use assert_matches::assert_matches;
use rand::{seq::SliceRandom, Rng, SeedableRng};
use rstest::rstest;
use starknet::{
    accounts::{Account, AccountError, AccountFactory, ConnectedAccount, OpenZeppelinAccountFactory},
    contract::ContractFactory,
};
use starknet::{
    accounts::{ExecutionEncoding, SingleOwnerAccount},
    signers::{LocalWallet, SigningKey},
};
use starknet_core::{
    types::{
        BlockId, BlockTag, Call, ContractClass, ExecuteInvocation, ExecutionResult, Felt, FunctionCall,
        MaybePendingBlockWithTxHashes, StarknetError, TransactionReceipt, TransactionReceiptWithBlockInfo,
        TransactionTrace,
    },
    utils::starknet_keccak,
};
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError, SequencerGatewayProvider};
use std::time::Duration;

const GAS_PRICE: u128 = 128;

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

struct SetupBuilder {
    setup: TestSetup,
    block_time: String,
    block_production_disabled: bool,
}

impl SetupBuilder {
    pub fn new(setup: TestSetup) -> Self {
        Self { setup, block_time: "2s".into(), block_production_disabled: false }
    }
    pub fn with_block_production_disabled(mut self, disabled: bool) -> Self {
        self.block_production_disabled = disabled;
        self
    }
    pub fn with_block_time(mut self, block_time: impl Into<String>) -> Self {
        self.block_time = block_time.into();
        self
    }

    fn sequencer_args(&self) -> impl Iterator<Item = String> {
        [
            "--devnet".into(),
            "--no-l1-sync".into(),
            "--l1-gas-price".into(),
            "0".into(),
            "--chain-config-path".into(),
            "test_devnet.yaml".into(),
            "--chain-config-override".into(),
            format!("block_time={}", self.block_time),
            "--gateway".into(),
        ]
        .into_iter()
        .chain(self.block_production_disabled.then_some("--no-block-production".into()))
    }

    pub async fn run(self) -> RunningTestSetup {
        match self.setup {
            SequencerOnly => self.run_single_node().await,
            FullNodeAndSequencer => self.run_full_node_and_sequencer().await,
            GatewayAndSequencer => self.run_gateway_and_sequencer().await,
        }
    }

    async fn run_single_node(self) -> RunningTestSetup {
        // sequencer
        let mut sequencer =
            MadaraCmdBuilder::new().label("sequencer").enable_gateway().args(self.sequencer_args()).run();
        sequencer.wait_for_sync_to(0).await;
        RunningTestSetup::SingleNode(sequencer)
    }

    async fn run_gateway_and_sequencer(self) -> RunningTestSetup {
        let mut sequencer = MadaraCmdBuilder::new()
            .label("sequencer")
            .enable_gateway()
            .args(self.sequencer_args().chain(["--gateway-trusted-add-transaction-endpoint".into()]))
            .run();
        sequencer.wait_for_sync_to(0).await; // wait until devnet genesis is deployed

        let mut gateway = MadaraCmdBuilder::new()
            .label("gateway")
            .enable_gateway()
            .args([
                "--full",
                "--no-l1-sync",
                "--l1-gas-price",
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
                &format!("{}/madara", sequencer.gateway_root_url.as_ref().unwrap()),
                "--gateway",
            ])
            .run();
        gateway.wait_for_sync_to(0).await; // wait until devnet genesis is synced

        RunningTestSetup::TwoNodes { _sequencer: sequencer, user_facing: gateway }
    }

    async fn run_full_node_and_sequencer(self) -> RunningTestSetup {
        let mut sequencer =
            MadaraCmdBuilder::new().label("sequencer").enable_gateway().args(self.sequencer_args()).run();
        sequencer.wait_for_sync_to(0).await;

        let mut full_node = MadaraCmdBuilder::new()
            .label("full_node")
            .enable_gateway()
            .args([
                "--full",
                "--no-l1-sync",
                "--l1-gas-price",
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
            ])
            .run();
        full_node.wait_for_sync_to(0).await;

        RunningTestSetup::TwoNodes { _sequencer: sequencer, user_facing: full_node }
    }
}

use TestSetup::*;

#[allow(clippy::large_enum_variant)]
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

    pub fn json_rpc(&self) -> JsonRpcClient<HttpTransport> {
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
            300,
        )
        .await
    }

    pub async fn expect_tx_receipt_successful(&self, tx_hash: Felt) -> TransactionReceiptWithBlockInfo {
        let r = self.expect_tx_receipt(tx_hash).await;
        assert_eq!(r.receipt.execution_result(), &ExecutionResult::Succeeded);
        r
    }

    /// Returns (block_n, tx index in block).
    /// If the tx is in pending, it will have to wait - since we can't know the block_n from pending.
    pub async fn get_tx_position(&self, tx_hash: Felt) -> (u64, u64) {
        let receipt = self.expect_tx_receipt(tx_hash).await;
        if receipt.block.is_pending() {
            wait_for_next_block(&self.json_rpc()).await;
        }
        let block_n = self.expect_tx_receipt(tx_hash).await.block.block_number().unwrap();
        let position = self
            .json_rpc()
            .get_block_with_tx_hashes(BlockId::Number(block_n))
            .await
            .unwrap()
            .transactions()
            .binary_search(&tx_hash)
            .unwrap() as _;

        (block_n, position)
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

async fn get_latest_block_n(provider: &(impl Provider + Send + Sync)) -> u64 {
    let MaybePendingBlockWithTxHashes::Block(b) =
        provider.get_block_with_tx_hashes(BlockId::Tag(BlockTag::Latest)).await.unwrap()
    else {
        unreachable!("block latest is pending")
    };
    b.block_number
}

async fn wait_for_next_block(provider: &(impl Provider + Send + Sync)) {
    let start = get_latest_block_n(provider).await;
    wait_for_cond(
        move || async move {
            let got = get_latest_block_n(provider).await;
            if got > start {
                return Ok(());
            }
            bail!("Block n not reached: start={start}, got={got}")
        },
        Duration::from_millis(500),
        300,
    )
    .await;
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer)]
#[case::gateway(GatewayAndSequencer)]
#[case::single_node(SequencerOnly)]
async fn normal_transfer(#[case] setup: TestSetup) {
    let setup = SetupBuilder::new(setup).run().await;

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P) {
        let account = setup.account(provider).await;

        let recipient = ACCOUNTS[2];
        let amount = 15;

        let init_balance = setup.get_balance(recipient).await;
        let init_nonce = setup.get_nonce(account.address()).await;

        let res = account
            .execute_v3(make_transfer_call(recipient, amount))
            .gas_price(GAS_PRICE) // we have to do this as gateway doesn't have the estimate_fee or get nonce method.
            .gas(30_000)
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
    perform_test(&setup, &setup.json_rpc()).await;
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer)]
#[case::gateway(GatewayAndSequencer)]
#[case::single_node(SequencerOnly)]
/// Test more transfers, with some concurrency, across some block boundaries
async fn more_transfers(#[case] setup: TestSetup) {
    let setup = SetupBuilder::new(setup).with_block_time("4s").run().await;

    async fn perform_test<P: Provider + Sync + Send>(
        setup: &RunningTestSetup,
        provider: &P,
        balances: &mut [Felt],
        nonces: &mut [Felt],
    ) {
        let n_accounts = nonces.len();
        let n_blocks = 2;
        let n_batch = 2;
        let n_txs = 4;
        let chain_id = setup.chain_id().await;

        // add a tiny bit of rng, but reproducible.
        // this is because futures join_all is not fair, it'll give some advantage to the first futures being joined
        let mut rng = rand::rngs::StdRng::seed_from_u64(21312);

        for _ in 0..n_blocks {
            for _ in 0..n_batch {
                let mut cases = nonces.iter_mut().zip(balances.iter_mut()).enumerate().collect::<Vec<_>>();
                cases.shuffle(&mut rng);
                let offset = rng.gen_range(0..n_accounts);
                futures::future::join_all(cases.into_iter().map(|(i, (nonce, bal))| async move {
                    let account = ACCOUNTS[i];
                    let recipient = ACCOUNTS[(i + offset) % n_accounts];

                    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRETS[i]));
                    let mut account =
                        SingleOwnerAccount::new(provider, signer, account, chain_id, ExecutionEncoding::New);
                    account.set_block_id(BlockId::Tag(BlockTag::Pending));

                    let mut hashes = vec![];
                    for _ in 0..n_txs {
                        let res = account
                            .execute_v3(make_transfer_call(recipient, 10))
                            .gas_price(GAS_PRICE)
                            .gas(30_000)
                            .nonce(*nonce)
                            .send()
                            .await
                            .unwrap();
                        hashes.push(res.transaction_hash);
                        *nonce += Felt::ONE;
                    }
                    for hash in hashes {
                        let receipt = setup.expect_tx_receipt_successful(hash).await;
                        let TransactionReceipt::Invoke(receipt) = receipt.receipt else {
                            unreachable!("got non-invoke receipt: {:?}", receipt)
                        };
                        *bal -= receipt.actual_fee.amount;
                    }
                }))
                .await;

                let got_bal = futures::future::join_all(
                    (0..n_accounts).map(|i| async move { setup.get_balance(ACCOUNTS[i]).await[0] }),
                )
                .await;

                for (i, (bal, got_bal)) in balances.iter().zip(got_bal.iter()).enumerate() {
                    assert_eq!(bal, got_bal, "Account {i}");
                }
            }

            wait_for_next_block(&provider).await
        }
    }

    let n_accounts = 4;
    let bal = setup.get_balance(ACCOUNTS[0]).await[0];
    let mut balances = vec![bal; n_accounts];
    let mut nonces = vec![Felt::ZERO; n_accounts];

    // via gateway
    perform_test(&setup, &setup.gateway_client().await, &mut balances, &mut nonces).await;

    // via rpc
    perform_test(&setup, &setup.json_rpc(), &mut balances, &mut nonces).await;
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
    let setup = SetupBuilder::new(setup)
        // disable block prod to be sure the tx is still in the mempool by the time we check again
        .with_block_production_disabled(!wait_for_initial_transfer)
        .with_block_time("500min")
        .run()
        .await;

    let init_nonce = setup.get_nonce(ACCOUNTS[0]).await;

    // do a tx to increment the account nonce
    let res = setup
        .account(setup.json_rpc())
        .await
        .execute_v3(make_transfer_call(ACCOUNTS[4], 1418283))
        .gas_price(GAS_PRICE)
        .gas(30_000)
        .send()
        .await
        .unwrap();
    if wait_for_initial_transfer {
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
    }

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P, invalid_nonce: Felt) {
        let account = setup.account(provider).await;
        let res = account
            .execute_v3(make_transfer_call(ACCOUNTS[2], 1232))
            .gas_price(GAS_PRICE)
            .gas(30_000)
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
    perform_test(&setup, &setup.json_rpc(), init_nonce).await;
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer)]
#[case::gateway_and_sequencer(GatewayAndSequencer)]
#[case::single_node(SequencerOnly)]
/// Duplicated txn hash in mempool. Note: if the txn is already in the chain, we would get a nonce mismatch error, not duplicate txn.
async fn duplicate_txn(#[case] setup: TestSetup) {
    let setup = SetupBuilder::new(setup).with_block_production_disabled(true).run().await;

    let nonce = setup.get_nonce(ACCOUNTS[0]).await;
    let call = make_transfer_call(ACCOUNTS[4], 1418283);

    let _res = setup
        .account(setup.json_rpc())
        .await
        .execute_v3(call.clone())
        .nonce(nonce)
        .gas_price(GAS_PRICE)
        .gas(30_000)
        .send()
        .await
        .unwrap();

    async fn perform_test<P: Provider + Sync + Send>(
        setup: &RunningTestSetup,
        provider: &P,
        nonce: Felt,
        call: Vec<Call>,
    ) {
        let account = setup.account(provider).await;
        let res = account.execute_v3(call).nonce(nonce).gas_price(GAS_PRICE).gas(30_000).send().await;
        assert_matches!(
            res.unwrap_err(),
            AccountError::Provider(ProviderError::StarknetError(StarknetError::DuplicateTx))
        );
    }

    // via gateway
    perform_test(&setup, &setup.gateway_client().await, nonce, call.clone()).await;

    // via rpc
    perform_test(&setup, &setup.json_rpc(), nonce, call.clone()).await;
}

#[tokio::test]
#[rstest]
#[case::full_node(FullNodeAndSequencer)]
#[case::gateway_and_sequencer(GatewayAndSequencer)]
#[case::single_node(SequencerOnly)]
/// Sometimes, the deploy_account tx arrives after the invoke_tx. Simulate this case.
async fn deploy_account_wrong_order_works(#[case] setup: TestSetup) {
    let setup = SetupBuilder::new(setup).with_block_time("500ms").run().await;

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P, salt: Felt) {
        let account = setup.account(provider).await;
        let nonce = setup.get_nonce(ACCOUNTS[0]).await;
        let oz_class_hash =
            setup.json_rpc().get_class_hash_at(BlockId::Tag(BlockTag::Pending), account.address()).await.unwrap(); // json-rpc only method

        let key = Felt::from_hex_unchecked("0x55523255");

        let mut factory = OpenZeppelinAccountFactory::new(
            oz_class_hash,
            setup.chain_id().await,
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(key)),
            provider,
        )
        .await
        .unwrap();
        factory.set_block_id(BlockId::Tag(BlockTag::Pending));

        let deploy = factory.deploy_v3(salt).nonce(Felt::ZERO).gas_price(GAS_PRICE).gas(0x10000);

        // Calculate contract address.
        let deployed_contract_address = deploy.address();

        // Give money to the address.
        let res = account
            .execute_v3(make_transfer_call(deployed_contract_address, 0x21536523))
            .nonce(nonce)
            .gas_price(GAS_PRICE)
            .gas(0x10000000000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        assert_eq!(setup.get_balance(deployed_contract_address).await[0], 0x21536523.into());

        // 1. Send money using the account.

        let mut deployed_account = SingleOwnerAccount::new(
            provider,
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(key)),
            deployed_contract_address,
            setup.chain_id().await,
            ExecutionEncoding::New,
        );
        deployed_account.set_block_id(BlockId::Tag(BlockTag::Pending));

        let recipient = Felt::from_hex_unchecked("0x8888");
        let amount = 0x11128;
        let res_invoke = deployed_account
            .execute_v3(make_transfer_call(recipient, amount))
            .nonce(Felt::ONE) // Note: Nonce is ONE here
            .gas_price(GAS_PRICE)
            .gas(0x10000)
            .send()
            .await
            .unwrap();

        // The txs are not there yet.

        assert_matches!(
            setup.json_rpc().get_transaction_receipt(res_invoke.transaction_hash).await.unwrap_err(),
            ProviderError::StarknetError(StarknetError::TransactionHashNotFound)
        );
        wait_for_next_block(&provider).await;
        assert_matches!(
            setup.json_rpc().get_transaction_receipt(res_invoke.transaction_hash).await.unwrap_err(),
            ProviderError::StarknetError(StarknetError::TransactionHashNotFound)
        );
        wait_for_next_block(&provider).await;
        assert_matches!(
            setup.json_rpc().get_transaction_receipt(res_invoke.transaction_hash).await.unwrap_err(),
            ProviderError::StarknetError(StarknetError::TransactionHashNotFound)
        );

        // 2. Send the deployaccount.
        let res_deploy = deploy.send().await.unwrap();

        // Now the txs should be there.

        setup.expect_tx_receipt_successful(res_deploy.transaction_hash).await;
        setup.expect_tx_receipt_successful(res_invoke.transaction_hash).await;

        // Assert the transaction order.

        let deploy_pos = setup.get_tx_position(res_deploy.transaction_hash).await;
        let invoke_pos = setup.get_tx_position(res_invoke.transaction_hash).await;

        assert!(
            deploy_pos < invoke_pos,
            "Wrong transaction order: deploy_pos={deploy_pos:?} should be before invoke_pos={invoke_pos:?}"
        );
    }

    // via gateway
    perform_test(&setup, &setup.gateway_client().await, Felt::THREE).await;

    // via rpc
    perform_test(&setup, &setup.json_rpc(), Felt::TWO).await;
}

#[tokio::test]
#[rstest]
#[case::full_node_rpc(FullNodeAndSequencer, false, false)]
#[case::full_node(FullNodeAndSequencer, false, true)]
#[case::full_node_rpc_no_pending(FullNodeAndSequencer, true, false)]
#[case::gateway_and_sequencer_rpc(GatewayAndSequencer, false, false)]
#[case::gateway_and_sequencer(GatewayAndSequencer, true, false)]
#[case::gateway_and_sequencer_no_pending(GatewayAndSequencer, false, true)]
#[case::single_node_rpc(SequencerOnly, false, false)]
#[case::single_node(SequencerOnly, false, true)]
/// Declare a contract, then
/// no_pending_block: when enabled, and wait_for_txs true, this means there can only be one tx per block. allows testing
///  that state rolls over block boundaries correctly.
async fn declare_sierra_then_deploy(
    #[case] setup: TestSetup,
    #[case] no_pending_block: bool,
    #[case] via_gateway_api: bool,
) {
    let setup =
        SetupBuilder::new(setup).with_block_time(if !no_pending_block { "500min" } else { "500ms" }).run().await;

    async fn perform_test<P: Provider + Sync + Send>(setup: &RunningTestSetup, provider: &P) {
        let mut nonce = setup.get_nonce(ACCOUNTS[0]).await;

        let sierra_class: starknet_core::types::contract::SierraClass =
            serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
        let flattened_class = sierra_class.clone().flatten().unwrap();
        let (compiled_class_hash, _compiled_class) =
            mp_class::FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm().unwrap();

        // 0. Declare a class.

        let account = setup.account(provider).await;
        let res = account
            .declare_v3(flattened_class.clone().into(), compiled_class_hash)
            .nonce(nonce)
            .gas_price(GAS_PRICE)
            .gas(0x10000000000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        nonce += Felt::ONE;
        let class_hash = res.class_hash;

        let res = provider.get_class(BlockId::Tag(BlockTag::Pending), class_hash).await.unwrap();
        let ContractClass::Sierra(class) = res else {
            unreachable!("class {class_hash:#x} expected to be sierra class")
        };
        assert_eq!(class, flattened_class);

        // 1. Deploy an account using the UDC.

        let key = SigningKey::from_secret_scalar(Felt::from_hex_unchecked("0x273623"));

        // Simulate to get the contract address.

        let res = ContractFactory::new(
            class_hash,
            setup.account(setup.json_rpc()).await, // simulate can only be done through json_rpc
        )
        .deploy_v3(vec![key.verifying_key().scalar()], /* salt */ Felt::TWO, /* unique */ true)
        .simulate(/* skip_validate */ false, /* charge_fee */ true)
        .await
        .unwrap();

        let TransactionTrace::Invoke(res) = res.transaction_trace else {
            unreachable!("transaction trace should be invoke")
        };
        let ExecuteInvocation::Success(res) = res.execute_invocation else {
            unreachable!("failed simulation: {:?}", res.execute_invocation)
        };
        let deployed_contract_address = res.result[2];
        assert_eq!(
            res.result,
            vec![
                /* results.len() */ Felt::ONE,
                /* results[0].len() */ Felt::ONE,
                /* results[0][0] */ deployed_contract_address
            ]
        );

        // Deploy the account.

        let res = ContractFactory::new(class_hash, account.clone())
            .deploy_v3(vec![key.verifying_key().scalar()], /* salt */ Felt::TWO, /* unique */ true)
            .nonce(nonce)
            .gas_price(GAS_PRICE)
            .gas(0x10000000000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        nonce += Felt::ONE;

        let mut deployed_account = SingleOwnerAccount::new(
            provider,
            LocalWallet::from_signing_key(key),
            deployed_contract_address,
            setup.chain_id().await,
            ExecutionEncoding::New,
        );
        deployed_account.set_block_id(BlockId::Tag(BlockTag::Pending));

        // Give money to the new account.

        let res = account
            .execute_v3(make_transfer_call(deployed_contract_address, 0x21536523))
            .nonce(nonce)
            .gas_price(GAS_PRICE)
            .gas(0x10000000000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        nonce += Felt::ONE;

        // Make a call with the new account.

        let recipient = Felt::from_hex_unchecked("0x128128");
        let amount = 0x128;
        let res = deployed_account
            .execute_v3(make_transfer_call(recipient, amount))
            .nonce(Felt::ZERO)
            .gas_price(GAS_PRICE)
            .gas(0x10000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        assert_eq!(setup.get_balance(recipient).await[0], amount.into());

        // 2. Counter-factual deploy. (DeployAccount txn)

        let key = SigningKey::from_secret_scalar(Felt::from_hex_unchecked("0x55555"));

        let mut factory = OpenZeppelinAccountFactory::new(
            class_hash,
            setup.chain_id().await,
            LocalWallet::from_signing_key(key),
            provider,
        )
        .await
        .unwrap();
        factory.set_block_id(BlockId::Tag(BlockTag::Pending));

        let deploy = factory.deploy_v3(/* salt */ Felt::THREE).nonce(Felt::ZERO).gas_price(GAS_PRICE).gas(0x10000);

        // Calculate contract address.
        let deployed_contract_address = deploy.address();

        // Give money to the address.
        let res = account
            .execute_v3(make_transfer_call(deployed_contract_address, 0x21536523))
            .nonce(nonce)
            .gas_price(GAS_PRICE)
            .gas(0x10000000000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        nonce += Felt::ONE;

        // Send deploy.
        deploy.send().await.unwrap();
        deployed_account.set_block_id(BlockId::Tag(BlockTag::Pending));

        // Do something with the account.
        let recipient = Felt::from_hex_unchecked("0x8888");
        let amount = 0x11128;
        let res = deployed_account
            .execute_v3(make_transfer_call(recipient, amount))
            .nonce(Felt::ONE) // Note: Nonce is ONE in case of counter-factual deploy.
            .gas_price(GAS_PRICE)
            .gas(0x10000)
            .send()
            .await
            .unwrap();
        setup.expect_tx_receipt_successful(res.transaction_hash).await;
        assert_eq!(setup.get_balance(recipient).await[0], amount.into());
    }

    // via gateway
    if via_gateway_api {
        perform_test(&setup, &setup.gateway_client().await).await;
    }
    // via rpc
    else {
        perform_test(&setup, &setup.json_rpc()).await;
    }
}
