use crate::{
    devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ACCOUNT_SECRETS, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::ensure;
use mp_class::FlattenedSierraClass;
use starknet::accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{
    BlockId, BlockTag, Call, Felt, FlattenedSierraClass as StarknetFlattenedSierraClass,
    MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedStateUpdate, StateDiff, TransactionReceiptWithBlockInfo,
};
use starknet_core::utils::starknet_keccak;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

const TX_COUNT_TARGET: usize = 100;
const MIN_BLOCK_HEIGHT_TARGET: u64 = 10;
const RECEIPT_WAIT_CONCURRENCY: usize = 20;
const SEND_TX_ATTEMPTS: u8 = 12;
const SEND_TX_RETRY_BASE_MS: u64 = 100;
const SEND_TX_RETRY_MAX_EXPONENT: u8 = 8;
const DEPLOY_ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x023f97ee39cb7032589df7f0ba66b92b8a4ed2ea67f9d2e31f2f8de4af61f14");
const DEPLOY_ACCOUNT_SALT: Felt = Felt::from_hex_unchecked("0x123");
const BLOCK_SIGNING_PRIVATE_KEY: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdea";
const TEST_BLOCK_TIME: &str = "1s";
const SLOW_BLOCK_TIME_FOR_MANUAL_CLOSE: &str = "60s";
const BLOCK_HASH_CONTRACT_ADDRESS: Felt = Felt::ONE;
const SYSTEM_CONFIG_CONTRACT_ADDRESS: Felt = Felt::TWO;

const L2_GAS_PRICE: u128 = 200_000;
const L2_GAS: u64 = 1_000_000_000;
const L1_GAS_PRICE: u128 = 128;
const L1_GAS: u64 = 30_000;
const L1_DATA_GAS_PRICE: u128 = 128;
const L1_DATA_GAS: u64 = 30_000;

macro_rules! with_devnet_fees {
    ($builder:expr) => {
        $builder
            .l2_gas_price(L2_GAS_PRICE)
            .l2_gas(L2_GAS)
            .l1_gas_price(L1_GAS_PRICE)
            .l1_gas(L1_GAS)
            .l1_data_gas_price(L1_DATA_GAS_PRICE)
            .l1_data_gas(L1_DATA_GAS)
    };
}

fn test_devnet_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_devnet.yaml").to_string_lossy().into_owned()
}

fn common_args(parallel_merkle: bool) -> Vec<String> {
    common_args_with_options(parallel_merkle, false, TEST_BLOCK_TIME)
}

fn common_args_with_options(parallel_merkle: bool, rpc_admin: bool, block_time: &str) -> Vec<String> {
    let mut args = vec![
        "--devnet".to_string(),
        "--no-l1-sync".to_string(),
        "--l1-gas-price".to_string(),
        "0".to_string(),
        "--blob-gas-price".to_string(),
        "0".to_string(),
        // This test sends transactions from ACCOUNT[0] and ACCOUNT[1], so both must be predeployed.
        "--devnet-contracts".to_string(),
        "2".to_string(),
        "--chain-config-path".to_string(),
        test_devnet_path(),
        "--chain-config-override".to_string(),
        format!("block_time={block_time}"),
        "--private-key".to_string(),
        BLOCK_SIGNING_PRIVATE_KEY.to_string(),
    ];

    if parallel_merkle {
        args.extend([
            "--parallel-merkle-enabled".to_string(),
            "--parallel-merkle-flush-interval".to_string(),
            "3".to_string(),
            "--parallel-merkle-max-inflight".to_string(),
            "10".to_string(),
            "--parallel-merkle-trie-log-mode".to_string(),
            "checkpoint".to_string(),
        ]);
    }

    if rpc_admin {
        args.extend(["--rpc-admin".to_string(), "--rpc-unsafe".to_string()]);
    }

    args
}

fn extract_block_hash(block: MaybePreConfirmedBlockWithTxHashes) -> Felt {
    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(block) => block.block_hash,
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            panic!("unexpected pre-confirmed block where confirmed block was expected")
        }
    }
}

#[derive(Debug, Clone)]
struct BlockStateChange {
    state_diff: StateDiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConfirmedBlockSnapshot {
    block_hash: Felt,
    state_root: Felt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SquashedStateDiff {
    storage_exact: BTreeMap<(Felt, Felt), Felt>,
    system_config_keys: BTreeSet<Felt>,
    nonces: BTreeMap<Felt, Felt>,
    deployed_or_replaced_classes: BTreeMap<Felt, Felt>,
    declared_classes: BTreeMap<Felt, Felt>,
    migrated_compiled_classes: BTreeMap<Felt, Felt>,
    deprecated_declared_classes: BTreeSet<Felt>,
}

impl SquashedStateDiff {
    fn apply_state_diff(&mut self, state_diff: &StateDiff) {
        for contract in &state_diff.storage_diffs {
            if contract.address == BLOCK_HASH_CONTRACT_ADDRESS {
                continue;
            }
            if contract.address == SYSTEM_CONFIG_CONTRACT_ADDRESS {
                for entry in &contract.storage_entries {
                    self.system_config_keys.insert(entry.key);
                }
                continue;
            }
            for entry in &contract.storage_entries {
                self.storage_exact.insert((contract.address, entry.key), entry.value);
            }
        }

        for nonce_update in &state_diff.nonces {
            self.nonces.insert(nonce_update.contract_address, nonce_update.nonce);
        }

        for deployed in &state_diff.deployed_contracts {
            self.deployed_or_replaced_classes.insert(deployed.address, deployed.class_hash);
        }

        for replaced in &state_diff.replaced_classes {
            self.deployed_or_replaced_classes.insert(replaced.contract_address, replaced.class_hash);
        }

        for declared in &state_diff.declared_classes {
            self.declared_classes.insert(declared.class_hash, declared.compiled_class_hash);
        }

        if let Some(migrated) = &state_diff.migrated_compiled_classes {
            for item in migrated {
                self.migrated_compiled_classes.insert(item.class_hash, item.compiled_class_hash);
            }
        }

        for class_hash in &state_diff.deprecated_declared_classes {
            self.deprecated_declared_classes.insert(*class_hash);
        }
    }
}

fn squash_state_diffs(changes: &[BlockStateChange]) -> SquashedStateDiff {
    let mut squashed = SquashedStateDiff::default();
    for change in changes {
        squashed.apply_state_diff(&change.state_diff);
    }
    squashed
}

fn make_transfer_call(to: Felt, amount: u64) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![to, Felt::from(amount), Felt::ZERO],
    }
}

fn format_error_chain(mut error: &(dyn Error + 'static)) -> String {
    let mut chain = String::new();
    let mut first = true;

    while let Some(source) = {
        if !first {
            chain.push_str(" -> ");
        }
        chain.push_str(&error.to_string());
        first = false;
        error.source()
    } {
        error = source;
    }

    if chain.is_empty() {
        "<no-chain>".to_string()
    } else {
        chain
    }
}

fn format_error_chain_debug(mut error: &(dyn Error + 'static)) -> String {
    let mut chain = String::new();
    let mut first = true;

    while let Some(source) = {
        if !first {
            chain.push_str(" -> ");
        }
        chain.push_str(&format!("{error:?}"));
        first = false;
        error.source()
    } {
        error = source;
    }

    if chain.is_empty() {
        "<no-chain>".to_string()
    } else {
        chain
    }
}

fn is_retriable_send_error(error: &(dyn Error + 'static)) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("transporterror")
        || message.contains("connecterror")
        || message.contains("connection refused")
        || message.contains("incompletemessage")
        || message.contains("error sending request")
        || message.contains("connection")
        || message.contains("timed out")
        || message.contains("timeout")
        || message.contains("broken pipe")
        || message.contains("connection reset")
        || message.contains("connection aborted")
        || message.contains("connection refused")
        || message.contains("io error")
}

fn transport_error_context(error: &(dyn Error + 'static)) -> String {
    if let Some(reqwest_error) = error.downcast_ref::<reqwest::Error>() {
        format!(
            "reqwest_error is_connect={} is_timeout={} is_status={} is_body={} url={:?}",
            reqwest_error.is_connect(),
            reqwest_error.is_timeout(),
            reqwest_error.is_status(),
            reqwest_error.is_body(),
            reqwest_error.url(),
        )
    } else {
        "non-reqwest transport error".to_string()
    }
}

async fn rpc_health_check(rpc: &JsonRpcClient<HttpTransport>) -> String {
    match rpc.block_hash_and_number().await {
        Ok(block) => format!("rpc healthy (block_n = {})", block.block_number),
        Err(error) => format!("rpc unhealthy ({error})"),
    }
}

fn retry_backoff_ms(attempt: u8) -> u64 {
    let expo = attempt.saturating_sub(1);
    SEND_TX_RETRY_BASE_MS.saturating_mul(2u64.pow((expo.min(SEND_TX_RETRY_MAX_EXPONENT)) as u32))
}

async fn send_tx_with_retry<T, E, Fut, F>(
    rpc: &JsonRpcClient<HttpTransport>,
    label: &str,
    max_attempts: u8,
    mut make_send: F,
) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Error + 'static,
{
    let mut attempt = 0u8;
    loop {
        match make_send().await {
            Ok(value) => return value,
            Err(error) => {
                attempt += 1;
                let is_retriable = is_retriable_send_error(&error);
                let root_cause_chain = format_error_chain(&error);
                let root_cause_chain_debug = format_error_chain_debug(&error);
                let transport_ctx = transport_error_context(&error);
                let health = rpc_health_check(rpc).await;

                eprintln!(
                    "❌ tx send failed for {label} (attempt {attempt}/{max_attempts})\n\
                    error: {error}\n\
                    chain: {root_cause_chain}\n\
                    chain_debug: {root_cause_chain_debug}\n\
                    transport_ctx: {transport_ctx}\n\
                    retriable: {is_retriable}\n\
                    rpc_health: {health}"
                );

                if !is_retriable || attempt >= max_attempts {
                    panic!(
                        "failed to send tx for {label} after {attempt} attempts: {error}. \
                         chain: {root_cause_chain}. rpc health: {health}"
                    );
                }

                let backoff_ms = retry_backoff_ms(attempt);
                eprintln!("↻ retrying {label} in {backoff_ms}ms");
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

async fn wait_for_confirmed_receipt<P: Provider + Sync>(
    provider: &P,
    tx_hash: Felt,
) -> TransactionReceiptWithBlockInfo {
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_block(), "tx not confirmed yet");
            Ok(receipt)
        },
        Duration::from_millis(250),
        400,
    )
    .await
}

async fn wait_for_block_at_least<P: Provider + Sync>(provider: &P, target_block_n: u64) {
    let _ = wait_for_cond(
        || async {
            let block_n = provider.block_hash_and_number().await?.block_number;
            ensure!(block_n >= target_block_n, "latest block is {block_n}, waiting for >= {target_block_n}");
            Ok(block_n)
        },
        Duration::from_millis(250),
        600,
    )
    .await;
}

async fn collect_confirmed_block_state_changes(rpc: &JsonRpcClient<HttpTransport>) -> Vec<BlockStateChange> {
    let latest_block_n = rpc.block_hash_and_number().await.unwrap().block_number;
    let mut changes = Vec::with_capacity((latest_block_n + 1) as usize);

    for block_n in 0..=latest_block_n {
        let update = rpc.get_state_update(BlockId::Number(block_n)).await.unwrap();
        let update = match update {
            MaybePreConfirmedStateUpdate::Update(update) => update,
            MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                panic!("unexpected pre-confirmed state update for block #{block_n}")
            }
        };

        changes.push(BlockStateChange { state_diff: update.state_diff });
    }

    changes
}

async fn start_node(parallel_merkle: bool) -> MadaraCmd {
    start_node_with_options(parallel_merkle, false, TEST_BLOCK_TIME).await
}

async fn start_node_with_options(parallel_merkle: bool, rpc_admin: bool, block_time: &str) -> MadaraCmd {
    let label = if parallel_merkle { "parallel-merkle-enabled" } else { "parallel-merkle-disabled" };
    let env_vars = vec![("RUST_LOG", "info")];

    for _ in 0..4 {
        let args = if !rpc_admin && block_time == TEST_BLOCK_TIME {
            common_args(parallel_merkle)
        } else {
            common_args_with_options(parallel_merkle, rpc_admin, block_time)
        };
        let mut node = MadaraCmdBuilder::new().label(label).env(env_vars.clone()).args(args).run_no_wait();

        node.hook_stdout_and_wait_for_ports(true, false, rpc_admin);
        node.wait_for_ready().await;
        node.wait_for_sync_to(0).await;

        let genesis_hash =
            extract_block_hash(node.json_rpc().get_block_with_tx_hashes(BlockId::Number(0)).await.unwrap());

        if genesis_hash != Felt::ZERO {
            return node;
        }

        node.stop();
        sleep(Duration::from_millis(250)).await;
    }

    panic!("could not start {} node with valid genesis hash", label)
}

async fn admin_rpc_call(admin_rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let client = reqwest::Client::new();
    let response = client
        .post(admin_rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        }))
        .send()
        .await
        .unwrap();
    let value = response.json::<serde_json::Value>().await.unwrap();
    if value.get("error").is_some() {
        panic!("admin rpc call `{method}` failed: {value}");
    }
    value
}

async fn admin_close_block(admin_rpc_url: &str) {
    let _ = admin_rpc_call(admin_rpc_url, "madara_closeBlock", serde_json::json!([])).await;
}

async fn admin_revert_to(admin_rpc_url: &str, block_hash: Felt) {
    let _ = admin_rpc_call(admin_rpc_url, "madara_revertTo", serde_json::json!([format!("0x{:x}", block_hash)])).await;
}

async fn admin_set_block_production(admin_rpc_url: &str, action: &str) {
    let _ = admin_rpc_call(admin_rpc_url, "madara_service", serde_json::json!([["block_production"], action])).await;
}

fn snapshot_from_state_update(block_n: u64, update: MaybePreConfirmedStateUpdate) -> ConfirmedBlockSnapshot {
    match update {
        MaybePreConfirmedStateUpdate::Update(update) => {
            ConfirmedBlockSnapshot { block_hash: update.block_hash, state_root: update.new_root }
        }
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("unexpected pre-confirmed state update for block #{block_n}")
        }
    }
}

async fn collect_confirmed_block_snapshots(
    rpc: &JsonRpcClient<HttpTransport>,
    from_block: u64,
    to_block: u64,
) -> BTreeMap<u64, ConfirmedBlockSnapshot> {
    let mut snapshots = BTreeMap::new();
    for block_n in from_block..=to_block {
        let update = rpc.get_state_update(BlockId::Number(block_n)).await.unwrap();
        snapshots.insert(block_n, snapshot_from_state_update(block_n, update));
    }
    snapshots
}

/// Sends all test transactions to a node, waits for confirmations, then collects all confirmed block state diffs.
async fn send_transactions_and_collect_block_state(
    rpc: Arc<JsonRpcClient<HttpTransport>>,
    flattened_class: StarknetFlattenedSierraClass,
    compiled_class_hash: Felt,
) -> Vec<BlockStateChange> {
    let chain_id = rpc.chain_id().await.unwrap();

    let mut account_a = SingleOwnerAccount::new(
        rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET)),
        ACCOUNT_ADDRESS,
        chain_id,
        ExecutionEncoding::New,
    );
    account_a.set_block_id(BlockId::Tag(BlockTag::Latest));

    let mut account_b = SingleOwnerAccount::new(
        rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRETS[1])),
        ACCOUNTS[1],
        chain_id,
        ExecutionEncoding::New,
    );
    account_b.set_block_id(BlockId::Tag(BlockTag::Latest));

    let oz_class_hash = rpc.get_class_hash_at(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();

    let mut account_factory = OpenZeppelinAccountFactory::new(
        oz_class_hash,
        chain_id,
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(DEPLOY_ACCOUNT_SECRET)),
        &rpc,
    )
    .await
    .unwrap();
    account_factory.set_block_id(BlockId::Tag(BlockTag::Latest));

    let deploy_address = account_factory.deploy_v3(DEPLOY_ACCOUNT_SALT).address();

    let mut nonce_a = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    let mut nonce_b = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNTS[1]).await.unwrap();

    // Fund the deploy account
    let max_attempts = SEND_TX_ATTEMPTS;
    let funding_result = send_tx_with_retry(&rpc, "funding deploy account", max_attempts, || async {
        let funding =
            with_devnet_fees!(account_a.execute_v3(vec![make_transfer_call(deploy_address, 1_000_000_000_000_000)]))
                .nonce(nonce_a);
        funding.send().await
    })
    .await;
    wait_for_confirmed_receipt(&*rpc, funding_result.transaction_hash).await;
    nonce_a += Felt::ONE;

    // Declare the test contract
    let declare_result = send_tx_with_retry(&rpc, "declare test contract", max_attempts, || async {
        let declare =
            with_devnet_fees!(account_a.declare_v3(flattened_class.clone().into(), compiled_class_hash).nonce(nonce_a));
        declare.send().await
    })
    .await;
    wait_for_confirmed_receipt(&*rpc, declare_result.transaction_hash).await;
    nonce_a += Felt::ONE;

    // Deploy the account
    let deploy_result = send_tx_with_retry(&rpc, "deploy account", max_attempts, || async {
        let deploy_tx = with_devnet_fees!(account_factory.deploy_v3(DEPLOY_ACCOUNT_SALT).nonce(Felt::ZERO));
        deploy_tx.send().await
    })
    .await;
    wait_for_confirmed_receipt(&*rpc, deploy_result.transaction_hash).await;

    // Send transfer transactions
    let mut transfer_tx_hashes = Vec::with_capacity(TX_COUNT_TARGET - 3);
    for step in 4..=TX_COUNT_TARGET {
        let recipient = ACCOUNTS[(step % (ACCOUNTS.len() - 1)) + 1];
        let amount = 10 + step as u64;
        let tx_hash = if step % 2 == 0 {
            let sender_nonce = nonce_a;
            let invoke_result = send_tx_with_retry(
                &rpc,
                &format!("transfer step {step} from acct0 to {recipient:#x} amount {amount}"),
                max_attempts,
                || async {
                    let invoke = with_devnet_fees!(account_a
                        .execute_v3(vec![make_transfer_call(recipient, amount)])
                        .nonce(sender_nonce));
                    invoke.send().await
                },
            )
            .await;
            nonce_a += Felt::ONE;
            invoke_result.transaction_hash
        } else {
            let sender_nonce = nonce_b;
            let invoke_result = send_tx_with_retry(
                &rpc,
                &format!("transfer step {step} from acct1 to {recipient:#x} amount {amount}"),
                max_attempts,
                || async {
                    let invoke = with_devnet_fees!(account_b
                        .execute_v3(vec![make_transfer_call(recipient, amount)])
                        .nonce(sender_nonce));
                    invoke.send().await
                },
            )
            .await;
            nonce_b += Felt::ONE;
            invoke_result.transaction_hash
        };

        transfer_tx_hashes.push(tx_hash);
    }

    // Wait for all transfer transactions to be confirmed
    for chunk in transfer_tx_hashes.chunks(RECEIPT_WAIT_CONCURRENCY) {
        let _receipts =
            futures::future::join_all(chunk.iter().copied().map(|tx_hash| wait_for_confirmed_receipt(&*rpc, tx_hash)))
                .await;
    }

    // Wait for target block height
    wait_for_block_at_least(&*rpc, MIN_BLOCK_HEIGHT_TARGET).await;

    collect_confirmed_block_state_changes(&rpc).await
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_merkle_and_sequential_match_squashed_state_diff_after_100_txs() {
    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class = sierra_class.flatten().unwrap();
    let compiled_hashes = FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm_with_hashes().unwrap();
    let compiled_class_hash = compiled_hashes.blake_hash;

    // --- Run sequential mode first ---
    let sequential = start_node(false).await;
    let seq_rpc = sequential.json_rpc();

    let seq_changes = send_transactions_and_collect_block_state(
        Arc::new(seq_rpc.clone()),
        flattened_class.clone(),
        compiled_class_hash,
    )
    .await;

    // Stop the sequential node before starting parallel
    drop(sequential);
    sleep(Duration::from_millis(500)).await;

    // --- Run parallel mode ---
    let parallel = start_node(true).await;
    let par_rpc = parallel.json_rpc();

    let par_changes =
        send_transactions_and_collect_block_state(Arc::new(par_rpc.clone()), flattened_class, compiled_class_hash)
            .await;

    // Stop the parallel node
    drop(parallel);

    let seq_squashed = squash_state_diffs(&seq_changes);
    let par_squashed = squash_state_diffs(&par_changes);

    // Compare squashed state while ignoring storage values for system contracts:
    // - 0x1 is ignored entirely
    // - 0x2 compares touched keys only (values intentionally ignored)
    assert_eq!(seq_squashed, par_squashed, "squashed state diffs diverged between sequential and parallel runs");
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_merkle_revert_to_non_checkpoint_block_restores_exact_state() {
    const TARGET_REVERT_BLOCK_N: u64 = 25;
    const FINAL_BLOCK_N: u64 = 50;

    let mut node = start_node_with_options(true, true, SLOW_BLOCK_TIME_FOR_MANUAL_CLOSE).await;
    let rpc = Arc::new(node.json_rpc());
    let admin_rpc_url = format!("{}rpc/v0.1.0/", node.rpc_admin_url());

    let chain_id = rpc.chain_id().await.unwrap();

    let mut account_a = SingleOwnerAccount::new(
        rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET)),
        ACCOUNT_ADDRESS,
        chain_id,
        ExecutionEncoding::New,
    );
    account_a.set_block_id(BlockId::Tag(BlockTag::Latest));

    let mut account_b = SingleOwnerAccount::new(
        rpc.clone(),
        LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRETS[1])),
        ACCOUNTS[1],
        chain_id,
        ExecutionEncoding::New,
    );
    account_b.set_block_id(BlockId::Tag(BlockTag::Latest));

    let mut nonce_a = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();
    let mut nonce_b = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNTS[1]).await.unwrap();

    for step in 1..=FINAL_BLOCK_N {
        let (tx_hash, sender_label) = if step % 2 == 0 {
            let sender_nonce = nonce_a;
            let result = send_tx_with_retry(
                &rpc,
                &format!("manual-close transfer block {step} from acct0 to acct1"),
                SEND_TX_ATTEMPTS,
                || async {
                    with_devnet_fees!(account_a
                        .execute_v3(vec![make_transfer_call(ACCOUNTS[1], 100 + step)])
                        .nonce(sender_nonce))
                    .send()
                    .await
                },
            )
            .await;
            nonce_a += Felt::ONE;
            (result.transaction_hash, "acct0")
        } else {
            let sender_nonce = nonce_b;
            let result = send_tx_with_retry(
                &rpc,
                &format!("manual-close transfer block {step} from acct1 to acct0"),
                SEND_TX_ATTEMPTS,
                || async {
                    with_devnet_fees!(account_b
                        .execute_v3(vec![make_transfer_call(ACCOUNT_ADDRESS, 100 + step)])
                        .nonce(sender_nonce))
                    .send()
                    .await
                },
            )
            .await;
            nonce_b += Felt::ONE;
            (result.transaction_hash, "acct1")
        };

        admin_close_block(&admin_rpc_url).await;
        wait_for_confirmed_receipt(&*rpc, tx_hash).await;

        let latest = rpc.block_hash_and_number().await.unwrap().block_number;
        assert!(
            latest >= step,
            "after forcing close with tx from {sender_label}, latest block should be >= {step}, got {latest}"
        );
    }

    wait_for_block_at_least(&*rpc, FINAL_BLOCK_N).await;
    let latest_pre_revert = rpc.block_hash_and_number().await.unwrap();
    assert_eq!(
        latest_pre_revert.block_number, FINAL_BLOCK_N,
        "manual close path should produce exactly {FINAL_BLOCK_N} blocks before revert"
    );

    let pre_revert_snapshots = collect_confirmed_block_snapshots(&rpc, 0, FINAL_BLOCK_N).await;
    let target_snapshot =
        *pre_revert_snapshots.get(&TARGET_REVERT_BLOCK_N).expect("target block snapshot should exist");

    admin_set_block_production(&admin_rpc_url, "stop").await;
    sleep(Duration::from_millis(250)).await;
    admin_revert_to(&admin_rpc_url, target_snapshot.block_hash).await;

    let next_block_n = TARGET_REVERT_BLOCK_N + 1;
    let target_after_revert = wait_for_cond(
        || async {
            let update = rpc.get_state_update(BlockId::Number(TARGET_REVERT_BLOCK_N)).await?;
            let snapshot = snapshot_from_state_update(TARGET_REVERT_BLOCK_N, update);
            ensure!(
                snapshot.block_hash == target_snapshot.block_hash,
                "block hash at target block changed after revert: expected {:#x}, got {:#x}",
                target_snapshot.block_hash,
                snapshot.block_hash
            );
            ensure!(
                rpc.get_state_update(BlockId::Number(next_block_n)).await.is_err(),
                "block #{next_block_n} should be absent after revert"
            );
            Ok(snapshot)
        },
        Duration::from_millis(100),
        300,
    )
    .await;
    assert_eq!(
        target_after_revert.block_hash, target_snapshot.block_hash,
        "latest block hash after revert should match target block hash"
    );
    assert_eq!(
        target_after_revert.state_root, target_snapshot.state_root,
        "state root at target block should match pre-revert value"
    );

    let post_revert_snapshots = collect_confirmed_block_snapshots(&rpc, 0, TARGET_REVERT_BLOCK_N).await;
    for block_n in 0..=TARGET_REVERT_BLOCK_N {
        let before = pre_revert_snapshots.get(&block_n).expect("snapshot before revert should exist");
        let after = post_revert_snapshots.get(&block_n).expect("snapshot after revert should exist");
        assert_eq!(before.block_hash, after.block_hash, "block hash mismatch at block #{block_n} after revert");
        assert_eq!(before.state_root, after.state_root, "state root mismatch at block #{block_n} after revert");
    }

    assert!(
        rpc.get_state_update(BlockId::Number(next_block_n)).await.is_err(),
        "block #{next_block_n} must not exist after revert"
    );
    assert!(
        rpc.get_block_with_tx_hashes(BlockId::Number(next_block_n)).await.is_err(),
        "block #{next_block_n} must be pruned from block lookup after revert"
    );

    node.stop();
}
