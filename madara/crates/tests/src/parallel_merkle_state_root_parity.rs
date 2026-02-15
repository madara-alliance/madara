use crate::{
    devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmdBuilder,
};
use anyhow::ensure;
use mp_class::FlattenedSierraClass;
use starknet::accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{
    BlockId, BlockTag, Call, Felt, MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedStateUpdate,
    TransactionReceiptWithBlockInfo,
};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use std::path::PathBuf;
use std::time::Duration;

const NON_EMPTY_BLOCKS_TARGET: usize = 100;
const DEPLOY_ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x023f97ee39cb7032589df7f0ba66b92b8a4ed2ea67f9d2e31f2f8de4af61f14");
const DEPLOY_ACCOUNT_SALT: Felt = Felt::from_hex_unchecked("0x123");
const BLOCK_SIGNING_PRIVATE_KEY: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdea";

const L2_GAS_PRICE: u128 = 200_000;
const L2_GAS: u64 = 1_000_000_000;
const L1_GAS_PRICE: u128 = 128;
const L1_GAS: u64 = 30_000;
const L1_DATA_GAS_PRICE: u128 = 128;
const L1_DATA_GAS: u64 = 30_000;

#[derive(Debug)]
struct WorkloadResult {
    final_block_n: u64,
    final_root: Felt,
    step_blocks: Vec<u64>,
    step_tx_hashes: Vec<Felt>,
    step_roots: Vec<Felt>,
    step_diffs_json: Vec<String>,
}

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
    let mut args = vec![
        "--devnet".to_string(),
        "--no-l1-sync".to_string(),
        "--l1-gas-price".to_string(),
        "0".to_string(),
        "--blob-gas-price".to_string(),
        "0".to_string(),
        "--devnet-contracts".to_string(),
        "1".to_string(),
        "--chain-config-path".to_string(),
        test_devnet_path(),
        "--chain-config-override".to_string(),
        "block_time=5min".to_string(),
        "--rpc-admin".to_string(),
        "--rpc-unsafe".to_string(),
        "--private-key".to_string(),
        BLOCK_SIGNING_PRIVATE_KEY.to_string(),
    ];

    if parallel_merkle {
        args.extend([
            "--parallel-merkle-enabled".to_string(),
            "--parallel-merkle-flush-interval".to_string(),
            "4".to_string(),
            "--parallel-merkle-max-inflight".to_string(),
            "16".to_string(),
            "--parallel-merkle-trie-log-mode".to_string(),
            "checkpoint".to_string(),
        ]);
    }

    args
}

fn extract_new_root(update: MaybePreConfirmedStateUpdate) -> Felt {
    match update {
        MaybePreConfirmedStateUpdate::Update(update) => update.new_root,
        MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
            panic!("unexpected pre-confirmed state update when confirmed update was expected")
        }
    }
}

fn extract_block_tx_hashes(block: MaybePreConfirmedBlockWithTxHashes) -> Vec<Felt> {
    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(block) => block.transactions,
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            panic!("unexpected pre-confirmed block where confirmed block was expected")
        }
    }
}

fn canonicalize_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for item in values.iter_mut() {
                canonicalize_json(item);
            }
            values.sort_by(|a, b| {
                let a = serde_json::to_string(a).expect("serialize canonical lhs");
                let b = serde_json::to_string(b).expect("serialize canonical rhs");
                a.cmp(&b)
            });
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                canonicalize_json(item);
            }
        }
        _ => {}
    }
}

fn canonical_state_diff_json(state_diff: &serde_json::Value) -> String {
    let mut value = state_diff.clone();
    canonicalize_json(&mut value);
    serde_json::to_string(&value).expect("serialize canonical state diff")
}

fn make_transfer_call(to: Felt, amount: u64) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![to, Felt::from(amount), Felt::ZERO],
    }
}

async fn call_admin_method(admin_url: &str, method: &str, params: serde_json::Value) {
    let response = reqwest::Client::new()
        .post(admin_url)
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
        panic!("admin {method} failed: {value}");
    }
}

async fn admin_close_block(admin_url: &str) {
    call_admin_method(admin_url, "madara_closeBlock", serde_json::json!([])).await;
}

async fn wait_for_executed_receipt<P: Provider + Sync>(provider: &P, tx_hash: Felt) -> TransactionReceiptWithBlockInfo {
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_pre_confirmed() || receipt.block.is_block(), "tx has not reached execution yet");
            Ok(receipt)
        },
        Duration::from_millis(250),
        240,
    )
    .await
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
        240,
    )
    .await
}

async fn submit_and_close<P: Provider + Sync>(provider: &P, admin_url: &str, tx_hash: Felt) -> u64 {
    let receipt = wait_for_executed_receipt(provider, tx_hash).await;
    if receipt.block.is_pre_confirmed() {
        admin_close_block(admin_url).await;
        return wait_for_confirmed_receipt(provider, tx_hash).await.block.block_number();
    }

    receipt.block.block_number()
}

async fn run_workload(builder: MadaraCmdBuilder) -> WorkloadResult {
    let sierra_class: SierraClass = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
    let flattened_class = sierra_class.flatten().unwrap();
    let compiled_hashes = FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm_with_hashes().unwrap();
    let compiled_class_hash = compiled_hashes.blake_hash;

    let mut madara = builder.run();
    madara.wait_for_ready().await;
    madara.wait_for_sync_to(0).await;

    let rpc = madara.json_rpc();
    let admin_url = format!("{}rpc/v0.1.0/", madara.rpc_admin_url());

    let chain_id = rpc.chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account = SingleOwnerAccount::new(rpc.clone(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

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

    let deploy = account_factory.deploy_v3(DEPLOY_ACCOUNT_SALT);
    let deploy_address = deploy.address();
    let mut nonce = rpc.get_nonce(BlockId::Tag(BlockTag::Latest), ACCOUNT_ADDRESS).await.unwrap();

    let funding_tx =
        with_devnet_fees!(account.execute_v3(vec![make_transfer_call(deploy_address, 1_000_000_000_000_000)]))
            .nonce(nonce)
            .send()
            .await
            .unwrap();
    let mut non_empty_blocks = vec![submit_and_close(&rpc, &admin_url, funding_tx.transaction_hash).await];
    nonce += Felt::ONE;

    let declare_tx =
        with_devnet_fees!(account.declare_v3(flattened_class.clone().into(), compiled_class_hash).nonce(nonce))
            .send()
            .await
            .unwrap();
    non_empty_blocks.push(submit_and_close(&rpc, &admin_url, declare_tx.transaction_hash).await);
    nonce += Felt::ONE;

    let deploy_tx = with_devnet_fees!(deploy.nonce(Felt::ZERO)).send().await.unwrap();
    non_empty_blocks.push(submit_and_close(&rpc, &admin_url, deploy_tx.transaction_hash).await);

    for step in 4..=NON_EMPTY_BLOCKS_TARGET {
        let recipient = ACCOUNTS[(step % (ACCOUNTS.len() - 1)) + 1];
        let amount = 10 + step as u64;
        let invoke_tx = with_devnet_fees!(account.execute_v3(vec![make_transfer_call(recipient, amount)]).nonce(nonce))
            .send()
            .await
            .unwrap();
        non_empty_blocks.push(submit_and_close(&rpc, &admin_url, invoke_tx.transaction_hash).await);
        nonce += Felt::ONE;
    }

    assert_eq!(non_empty_blocks.len(), NON_EMPTY_BLOCKS_TARGET);
    non_empty_blocks.sort_unstable();
    non_empty_blocks.dedup();
    assert_eq!(
        non_empty_blocks.len(),
        NON_EMPTY_BLOCKS_TARGET,
        "expected each transaction to land in a distinct non-empty block"
    );

    let final_block_n = *non_empty_blocks.last().expect("at least one block should exist");
    let mut step_blocks = Vec::with_capacity(NON_EMPTY_BLOCKS_TARGET);
    let mut step_tx_hashes = Vec::with_capacity(NON_EMPTY_BLOCKS_TARGET);
    let mut step_roots = Vec::with_capacity(NON_EMPTY_BLOCKS_TARGET);
    let mut step_diffs_json = Vec::with_capacity(NON_EMPTY_BLOCKS_TARGET);

    for &block_n in &non_empty_blocks {
        let block_tx_hashes =
            extract_block_tx_hashes(rpc.get_block_with_tx_hashes(BlockId::Number(block_n)).await.unwrap());
        assert_eq!(
            block_tx_hashes.len(),
            1,
            "expected exactly one transaction in each non-empty block, got {} at block {}",
            block_tx_hashes.len(),
            block_n
        );

        let update = rpc.get_state_update(BlockId::Number(block_n)).await.unwrap();
        let (root, diff_json) = match update {
            MaybePreConfirmedStateUpdate::Update(update) => (
                update.new_root,
                canonical_state_diff_json(
                    &serde_json::to_value(&update.state_diff).expect("serializing state diff for canonicalization"),
                ),
            ),
            MaybePreConfirmedStateUpdate::PreConfirmedUpdate(_) => {
                panic!("unexpected pre-confirmed state update when confirmed update was expected")
            }
        };

        step_blocks.push(block_n);
        step_tx_hashes.push(block_tx_hashes[0]);
        step_roots.push(root);
        step_diffs_json.push(diff_json);
    }

    let final_root = extract_new_root(rpc.get_state_update(BlockId::Number(final_block_n)).await.unwrap());
    WorkloadResult { final_block_n, final_root, step_blocks, step_tx_hashes, step_roots, step_diffs_json }
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_merkle_and_sequential_match_state_root_after_100_non_empty_blocks() {
    let mut seed = MadaraCmdBuilder::new()
        .label("parallel-merkle-seed")
        .env([("RUST_LOG", "info")])
        .args(common_args(false))
        .run();
    seed.wait_for_ready().await;
    seed.wait_for_sync_to(0).await;
    seed.stop();
    let seed_db = seed.db_dir().to_path_buf();

    let sequential_builder = MadaraCmdBuilder::new()
        .label("parallel-merkle-disabled")
        .env([("RUST_LOG", "info")])
        .clone_db_from(&seed_db)
        .args(common_args(false));
    let sequential = run_workload(sequential_builder).await;

    let parallel_builder = MadaraCmdBuilder::new()
        .label("parallel-merkle-enabled")
        .env([("RUST_LOG", "info")])
        .clone_db_from(&seed_db)
        .args(common_args(true));
    let parallel = run_workload(parallel_builder).await;

    for (idx, (seq_tx_hash, par_tx_hash)) in
        sequential.step_tx_hashes.iter().zip(parallel.step_tx_hashes.iter()).enumerate()
    {
        assert_eq!(seq_tx_hash, par_tx_hash, "tx hash diverged at step {} (1-based block position {})", idx, idx + 1);
    }

    for (idx, (seq_diff, par_diff)) in
        sequential.step_diffs_json.iter().zip(parallel.step_diffs_json.iter()).enumerate()
    {
        assert_eq!(
            seq_diff,
            par_diff,
            "state diff diverged at step {} (1-based block position {}), sequential_block={}, parallel_block={}",
            idx,
            idx + 1,
            sequential.step_blocks[idx],
            parallel.step_blocks[idx]
        );
    }

    for (idx, (seq_root, par_root)) in sequential.step_roots.iter().zip(parallel.step_roots.iter()).enumerate() {
        assert_eq!(
            seq_root,
            par_root,
            "state root diverged at step {} (1-based block position {}): sequential_block={} parallel_block={}",
            idx,
            idx + 1,
            sequential.step_blocks[idx],
            parallel.step_blocks[idx]
        );
    }

    assert_eq!(
        parallel.final_root, sequential.final_root,
        "state roots diverged after {NON_EMPTY_BLOCKS_TARGET} non-empty blocks: sequential block {} root {:#x}, parallel block {} root {:#x}",
        sequential.final_block_n,
        sequential.final_root,
        parallel.final_block_n,
        parallel.final_root
    );
}
