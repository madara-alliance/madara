use crate::{
    devnet::{ACCOUNTS, ACCOUNT_SECRETS, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::{bail, ensure};
use mc_class_exec::config::NativeConfig;
use mc_db::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig};
use mp_chain_config::ChainConfig;
use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{
    BlockId, BlockTag, Call, Felt, MaybePreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxHashes,
    TransactionReceiptWithBlockInfo,
};
use starknet_core::utils::starknet_keccak;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const MAX_GAS_PRICE_BOUND: u128 = 1_000_000;

fn devnet_chain_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_devnet.yaml")
}

fn devnet_args() -> Vec<String> {
    vec![
        "--devnet".into(),
        "--no-l1-sync".into(),
        "--l1-gas-price".into(),
        "0".into(),
        "--blob-gas-price".into(),
        "0".into(),
        "--chain-config-path".into(),
        devnet_chain_config_path().display().to_string(),
        "--chain-config-override".into(),
        "block_time=5min".into(),
        "--rpc-admin".into(),
        "--rpc-unsafe".into(),
    ]
}

async fn start_node(builder: MadaraCmdBuilder) -> MadaraCmd {
    let mut node = builder.run();
    node.wait_for_ready().await;
    node
}

async fn build_account_for(
    node: &MadaraCmd,
    address: Felt,
    secret: Felt,
) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
    let chain_id = node.json_rpc().chain_id().await.unwrap();
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(secret));
    let mut account = SingleOwnerAccount::new(node.json_rpc(), signer, address, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    account
}

fn transfer_call(recipient: Felt, amount_low: Felt) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount_low, Felt::ZERO],
    }
}

async fn submit_transfer_txs(node: &MadaraCmd, start_account_index: usize, count: usize) -> Vec<Felt> {
    assert!(start_account_index + count <= ACCOUNTS.len(), "not enough prefunded accounts for test");
    let mut tx_hashes = Vec::with_capacity(count);

    for index in 0..count {
        let sender_index = start_account_index + index;
        let recipient_index = (sender_index + 1) % ACCOUNTS.len();
        let account = build_account_for(node, ACCOUNTS[sender_index], ACCOUNT_SECRETS[sender_index]).await;
        let tx = account
            .execute_v3(vec![transfer_call(ACCOUNTS[recipient_index], Felt::from((index + 1) as u64))])
            .l1_gas(1_000_000)
            .l1_gas_price(MAX_GAS_PRICE_BOUND)
            .l2_gas(2_000_000)
            .l2_gas_price(MAX_GAS_PRICE_BOUND)
            .l1_data_gas(1_000_000)
            .l1_data_gas_price(MAX_GAS_PRICE_BOUND)
            .tip(0)
            .send()
            .await
            .unwrap();
        tx_hashes.push(tx.transaction_hash);
    }

    tx_hashes
}

async fn wait_for_preconfirmed_receipt(node: &MadaraCmd, tx_hash: Felt) -> TransactionReceiptWithBlockInfo {
    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_pre_confirmed(), "tx not preconfirmed yet");
            Ok(receipt)
        },
        Duration::from_millis(500),
        120,
    )
    .await
}

async fn wait_for_confirmed_receipt(node: &MadaraCmd, tx_hash: Felt) -> TransactionReceiptWithBlockInfo {
    wait_for_cond(
        || async {
            let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
            ensure!(receipt.block.is_block(), "tx not confirmed yet");
            Ok(receipt)
        },
        Duration::from_millis(500),
        120,
    )
    .await
}

async fn latest_confirmed_block(node: &MadaraCmd) -> (u64, Felt) {
    let MaybePreConfirmedBlockWithTxHashes::Block(block) =
        node.json_rpc().get_block_with_tx_hashes(BlockId::Tag(BlockTag::Latest)).await.unwrap()
    else {
        unreachable!("latest block should be confirmed")
    };

    (block.block_number, block.block_hash)
}

async fn wait_for_latest_confirmed_block(node: &MadaraCmd, expected_block_n: u64) -> Felt {
    wait_for_cond(
        || async {
            let (block_n, block_hash) = latest_confirmed_block(node).await;
            ensure!(block_n == expected_block_n, "expected latest confirmed {expected_block_n}, got {block_n}");
            Ok(block_hash)
        },
        Duration::from_millis(500),
        120,
    )
    .await
}

async fn wait_for_preconfirmed_block(
    node: &MadaraCmd,
    expected_block_n: u64,
    expected_tx_count: usize,
) -> PreConfirmedBlockWithTxHashes {
    wait_for_cond(
        || async {
            match node.json_rpc().get_block_with_tx_hashes(BlockId::Tag(BlockTag::PreConfirmed)).await? {
                MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(block) => {
                    ensure!(
                        block.block_number == expected_block_n,
                        "expected block #{expected_block_n}, got #{}",
                        block.block_number
                    );
                    ensure!(
                        block.transactions.len() == expected_tx_count,
                        "expected {expected_tx_count} tx(s), got {}",
                        block.transactions.len()
                    );
                    Ok(block)
                }
                MaybePreConfirmedBlockWithTxHashes::Block(_) => bail!("preconfirmed tag returned a confirmed block"),
            }
        },
        Duration::from_millis(500),
        120,
    )
    .await
}

fn admin_url(node: &MadaraCmd) -> String {
    format!("{}rpc/v0.1.0/", node.rpc_admin_url())
}

async fn admin_call(admin_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
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
    assert!(value.get("error").is_none(), "admin call failed: {value}");
    value
}

async fn admin_close_block(admin_url: &str) {
    let _ = admin_call(admin_url, "madara_closeBlock", serde_json::json!([])).await;
}

async fn admin_revert_to(admin_url: &str, block_hash: Felt) {
    let _ = admin_call(admin_url, "madara_revertToAndShutdown", serde_json::json!([format!("0x{block_hash:x}")])).await;
}

async fn wait_for_shutdown(node: &MadaraCmd) {
    wait_for_cond(
        || async {
            match node.json_rpc().block_hash_and_number().await {
                Ok(_) => Err(anyhow::anyhow!("node still running after revertToAndShutdown")),
                Err(_) => Ok(()),
            }
        },
        Duration::from_millis(200),
        120,
    )
    .await;
}

fn open_backend(base_path: &Path) -> Arc<MadaraBackend> {
    let builder = NativeConfig::builder();
    mc_class_exec::init_compilation_semaphore(builder.max_concurrent_compilations());
    let cairo_native_config = Arc::new(builder.build());
    let chain_config = Arc::new(ChainConfig::from_yaml(&devnet_chain_config_path()).unwrap());

    MadaraBackend::open_rocksdb(
        base_path,
        chain_config,
        MadaraBackendConfig { save_preconfirmed: true, skip_migration_backup: true, ..Default::default() },
        RocksDBConfig::default(),
        cairo_native_config,
    )
    .unwrap()
}

async fn create_confirmed_block(node: &MadaraCmd, admin_url: &str) -> (u64, Felt) {
    let first_tx = submit_transfer_txs(node, 0, 1).await.pop().unwrap();
    let _ = wait_for_preconfirmed_receipt(node, first_tx).await;
    admin_close_block(admin_url).await;
    let _ = wait_for_confirmed_receipt(node, first_tx).await;
    latest_confirmed_block(node).await
}

#[rstest]
#[case::empty(0)]
#[case::with_transactions(3)]
#[tokio::test]
async fn revert_to_latest_confirmed_clears_open_preconfirmed_block_from_db(#[case] open_preconfirmed_txs: usize) {
    let builder = MadaraCmdBuilder::new().args(devnet_args());
    let node = start_node(builder.clone()).await;
    let admin_url = admin_url(&node);

    let (target_block_n, target_block_hash) = create_confirmed_block(&node, &admin_url).await;
    assert_eq!(target_block_n, 1);

    if open_preconfirmed_txs > 0 {
        for tx_hash in submit_transfer_txs(&node, 1, open_preconfirmed_txs).await {
            let _ = wait_for_preconfirmed_receipt(&node, tx_hash).await;
        }
    }

    let _ = wait_for_preconfirmed_block(&node, target_block_n + 1, open_preconfirmed_txs).await;

    let db_dir = node.db_dir().to_path_buf();
    admin_revert_to(&admin_url, target_block_hash).await;
    wait_for_shutdown(&node).await;
    drop(node);

    let backend = open_backend(&db_dir);
    assert_eq!(backend.latest_confirmed_block_n(), Some(target_block_n));
    assert!(!backend.has_preconfirmed_block(), "preconfirmed block should be cleared from the DB");
}

#[tokio::test]
async fn revert_to_latest_confirmed_with_empty_preconfirmed_restarts_with_a_fresh_empty_preconfirmed_block() {
    let builder = MadaraCmdBuilder::new().args(devnet_args());
    let node = start_node(builder.clone()).await;
    let admin_url = admin_url(&node);

    let (target_block_n, target_block_hash) = create_confirmed_block(&node, &admin_url).await;
    let _ = wait_for_preconfirmed_block(&node, target_block_n + 1, 0).await;

    admin_revert_to(&admin_url, target_block_hash).await;
    wait_for_shutdown(&node).await;
    drop(node);

    let mut node = start_node(builder).await;
    let _ = wait_for_latest_confirmed_block(&node, target_block_n).await;
    let restarted_preconfirmed = wait_for_preconfirmed_block(&node, target_block_n + 1, 0).await;
    assert_eq!(restarted_preconfirmed.block_number, target_block_n + 1);
    assert!(restarted_preconfirmed.transactions.is_empty());

    node.stop();
}
