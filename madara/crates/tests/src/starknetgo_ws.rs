use crate::{
    devnet::{ACCOUNTS, ACCOUNT_SECRETS, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::ensure;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, Felt};
use starknet_core::utils::starknet_keccak;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[tokio::test]
async fn starknetgo_ws_devnet_subscriptions() {
    let mut node = MadaraCmdBuilder::new()
        .args([
            "--devnet",
            "--no-l1-sync",
            "--rpc-admin",
            "--l1-gas-price",
            "0",
            "--blob-gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            "block_time=5min",
        ])
        .run();
    node.wait_for_ready().await;

    for version in ["v0_10", "v0_10_2"] {
        run_starknetgo_matrix(&node, version).await;
    }
}

async fn run_starknetgo_matrix(node: &MadaraCmd, version: &str) {
    let repo_root = repo_root();
    let coordination_dir = TempDir::with_prefix(format!("madara-starknetgo-{version}-")).unwrap();
    let ready_file = coordination_dir.path().join("ready");
    let status_ready_file = coordination_dir.path().join("status_ready");
    let tx_file = coordination_dir.path().join("txs.json");
    let rpc_url = versioned_rpc_url(node, version, "http");
    let ws_url = versioned_rpc_url(node, version, "ws");

    let mut child = Command::new("go")
        .arg("test")
        .arg(".")
        .arg("-run")
        .arg("TestMadaraWebsocketSubscriptions")
        .arg("-count=1")
        .arg("-timeout=120s")
        .current_dir(repo_root.join("tests/starknetgo_ws"))
        .env("MADARA_HTTP_URL", rpc_url)
        .env("MADARA_WS_URL", ws_url)
        .env("MADARA_READY_FILE", &ready_file)
        .env("MADARA_STATUS_READY_FILE", &status_ready_file)
        .env("MADARA_TX_FILE", &tx_file)
        .env("MADARA_ACCOUNT_0", felt_hex(ACCOUNTS[0]))
        .env("MADARA_ACCOUNT_1", felt_hex(ACCOUNTS[1]))
        .env("MADARA_ERC20_ADDRESS", felt_hex(ERC20_STRK_CONTRACT_ADDRESS))
        .env("MADARA_TRANSFER_KEY", felt_hex(starknet_keccak(b"Transfer")))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn go test for StarknetGo WS compatibility");

    wait_for_file_or_child(&ready_file, &mut child, "StarknetGo subscriptions to be ready");

    let hashes = submit_two_account_transactions(node).await;
    std::fs::write(&tx_file, format!("{{\"hashes\":[\"{}\",\"{}\"]}}\n", felt_hex(hashes.0), felt_hex(hashes.1)))
        .unwrap();

    wait_for_file_or_child(&status_ready_file, &mut child, "StarknetGo transaction-status subscriptions to be ready");

    wait_for_preconfirmed_receipt(node, hashes.0).await;
    wait_for_preconfirmed_receipt(node, hashes.1).await;
    close_block(node).await;
    wait_for_confirmed_receipt(node, hashes.0).await;
    wait_for_confirmed_receipt(node, hashes.1).await;

    let status = child.wait().expect("failed to wait for StarknetGo WS compatibility test");
    assert!(status.success(), "StarknetGo WS compatibility test failed for {version}: {status}");
}

async fn submit_two_account_transactions(node: &MadaraCmd) -> (Felt, Felt) {
    let chain_id = node.json_rpc().chain_id().await.unwrap();
    let account0 = build_account(node.json_rpc(), chain_id, ACCOUNTS[0], ACCOUNT_SECRETS[0]);
    let account1 = build_account(node.json_rpc(), chain_id, ACCOUNTS[1], ACCOUNT_SECRETS[1]);
    let (l1_price, l2_price, l1_data_price) = fetch_gas_prices(node).await;

    let success = account0
        .execute_v3(vec![transfer_call(ACCOUNTS[2], Felt::ONE, Felt::ZERO)])
        .l1_gas(1_000_000)
        .l1_gas_price(l1_price)
        .l2_gas(2_000_000)
        .l2_gas_price(l2_price)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(l1_data_price)
        .tip(0)
        .send()
        .await
        .unwrap();

    let revert = account1
        .execute_v3(vec![transfer_call(ACCOUNTS[2], Felt::ZERO, Felt::ONE)])
        .l1_gas(1_000_000)
        .l1_gas_price(l1_price)
        .l2_gas(2_000_000)
        .l2_gas_price(l2_price)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(l1_data_price)
        .tip(0)
        .send()
        .await
        .unwrap();

    (success.transaction_hash, revert.transaction_hash)
}

fn build_account(
    provider: JsonRpcClient<HttpTransport>,
    chain_id: Felt,
    address: Felt,
    secret: Felt,
) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(secret));
    let mut account = SingleOwnerAccount::new(provider, signer, address, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    account
}

fn transfer_call(recipient: Felt, amount_low: Felt, amount_high: Felt) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount_low, amount_high],
    }
}

async fn fetch_gas_prices(node: &MadaraCmd) -> (u128, u128, u128) {
    let block = node.json_rpc().get_block_with_tx_hashes(BlockId::Tag(BlockTag::Latest)).await.unwrap();
    (
        block.l1_gas_price().price_in_fri.try_into().unwrap(),
        block.l2_gas_price().price_in_fri.try_into().unwrap(),
        block.l1_data_gas_price().price_in_fri.try_into().unwrap(),
    )
}

async fn close_block(node: &MadaraCmd) {
    let response = reqwest::Client::new()
        .post(node.rpc_admin_url())
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "madara_closeBlock",
            "params": [],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    assert!(response.get("error").is_none(), "madara_closeBlock failed: {response}");
}

async fn wait_for_preconfirmed_receipt(node: &MadaraCmd, tx_hash: Felt) {
    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_pre_confirmed(), "tx not preconfirmed yet");
            Ok(())
        },
        Duration::from_millis(500),
        120,
    )
    .await;
}

async fn wait_for_confirmed_receipt(node: &MadaraCmd, tx_hash: Felt) {
    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_block(), "tx not confirmed yet");
            Ok(())
        },
        Duration::from_millis(500),
        120,
    )
    .await;
}

fn wait_for_file_or_child(path: &Path, child: &mut Child, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = child.try_wait().expect("failed to check StarknetGo test status") {
            panic!("{label} failed before readiness marker was written: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("timed out waiting for {label}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn versioned_rpc_url(node: &MadaraCmd, version: &str, scheme: &str) -> String {
    node.rpc_url().replacen("http", scheme, 1) + "rpc/" + version
}

fn felt_hex(value: Felt) -> String {
    format!("{value:#x}")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().to_path_buf()
}
