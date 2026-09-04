use crate::{
    devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::{anyhow, bail, Context};
use rstest::rstest;
use serde_json::{json, Value};
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Felt, MaybePreConfirmedBlockWithTxHashes};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use std::time::Duration;

const GAS_PRICE: u128 = 100000;
const TXS_PER_BLOCK: usize = 10;
const BLOCK_COUNT: usize = 20;
const TOTAL_TXS: usize = TXS_PER_BLOCK * BLOCK_COUNT;

fn make_transfer_call(recipient: Felt, amount: u128) -> Vec<starknet_core::types::Call> {
    vec![starknet_core::types::Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount.into(), Felt::ZERO],
    }]
}

async fn admin_rpc_call(node: &MadaraCmd, method: &str, params: Value) -> anyhow::Result<Value> {
    let endpoint = format!("{}rpc/v0_1_0", node.rpc_admin_url());
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let response = reqwest::Client::new()
        .post(&endpoint)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("Admin RPC request failed for method `{method}`"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .with_context(|| format!("Admin RPC response was not valid JSON for method `{method}`"))?;

    if !status.is_success() {
        bail!("Admin RPC HTTP error on `{method}`: status={status}, body={body}");
    }

    if let Some(error) = body.get("error") {
        bail!("Admin RPC `{method}` failed: {error}");
    }

    body.get("result").cloned().ok_or_else(|| anyhow!("Missing `result` in Admin RPC response: {body}"))
}

async fn set_replay_boundary(
    node: &MadaraCmd,
    block_n: u64,
    expected_tx_count: u64,
    last_tx_hash: Felt,
) -> anyhow::Result<Value> {
    admin_rpc_call(
        node,
        "madara_setReplayBoundary",
        json!([{
            "block_n": block_n,
            "expected_tx_count": expected_tx_count,
            "last_tx_hash": format!("{last_tx_hash:#x}"),
        }]),
    )
    .await
}

async fn get_replay_boundary_status(node: &MadaraCmd, block_n: u64) -> anyhow::Result<Option<Value>> {
    let result = admin_rpc_call(node, "madara_getReplayBoundaryStatus", json!([block_n])).await?;
    if result.is_null() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

async fn bypass_add_invoke_transaction(node: &MadaraCmd, invoke_tx: Value) -> anyhow::Result<Felt> {
    let result = admin_rpc_call(node, "madara_bypassAddInvokeTransaction", json!([invoke_tx])).await?;
    let tx_hash_str = result
        .get("transaction_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing `transaction_hash` in bypass response: {result}"))?;
    Felt::from_hex(tx_hash_str).map_err(|err| anyhow!("Invalid tx hash in bypass response `{tx_hash_str}`: {err}"))
}

/// Starts a paused-mempool replay node whose blocks close only at replay boundaries.
/// The long block time prevents the normal timer from influencing the test.
async fn start_replay_node() -> MadaraCmd {
    let mut node = MadaraCmdBuilder::new()
        .args([
            "--devnet",
            "--no-l1-sync",
            "--l1-gas-price",
            "0",
            "--blob-gas-price",
            "0",
            "--chain-config-path",
            "test_devnet.yaml",
            "--chain-config-override",
            "block_time=500min",
            "--rpc-admin",
            "--rpc-unsafe",
            "--replay-mode",
            "--mempool-paused",
        ])
        .run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(0).await;
    node
}

/// Builds and signs the deterministic transfer sequence without submitting it.
/// Returned hashes are the boundary oracle and requests are sent later through bypass RPC.
async fn prepare_replay_transactions(node: &MadaraCmd) -> (u64, Vec<Felt>, Vec<Value>) {
    let chain_id = node.json_rpc().chain_id().await.unwrap();
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    let start_block_n = node.json_rpc().block_hash_and_number().await.unwrap().block_number + 1;
    let mut next_nonce =
        node.json_rpc().get_nonce(BlockId::Tag(BlockTag::PreConfirmed), ACCOUNT_ADDRESS).await.unwrap();
    let mut tx_hashes = Vec::with_capacity(TOTAL_TXS);
    let mut invoke_txs = Vec::with_capacity(TOTAL_TXS);

    for tx_idx in 0..TOTAL_TXS {
        let recipient = ACCOUNTS[(tx_idx % (ACCOUNTS.len() - 1)) + 1];
        let prepared = account
            .execute_v3(make_transfer_call(recipient, 1))
            .nonce(next_nonce)
            .l2_gas_price(200000)
            .l2_gas(1_000_000_000_000)
            .l1_gas_price(GAS_PRICE)
            .l1_gas(30_000)
            .l1_data_gas_price(GAS_PRICE)
            .l1_data_gas(30_000)
            .tip(0)
            .prepared()
            .unwrap();
        tx_hashes.push(prepared.transaction_hash(false));
        invoke_txs.push(serde_json::to_value(prepared.get_invoke_request(false, false).await.unwrap()).unwrap());
        next_nonce += Felt::ONE;
    }
    (start_block_n, tx_hashes, invoke_txs)
}

/// Seeds every replay boundary before any bypass transaction is submitted.
/// Each block is capped at the configured count and anchored by its final transaction hash.
async fn seed_replay_boundaries(node: &MadaraCmd, start_block_n: u64, tx_hashes: &[Felt]) {
    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let last_tx_hash = tx_hashes[(block_idx + 1) * TXS_PER_BLOCK - 1];
        let status = set_replay_boundary(node, block_n, TXS_PER_BLOCK as u64, last_tx_hash).await.unwrap();
        assert_eq!(status["block_n"], json!(block_n));
        assert_eq!(status["expected_tx_count"], json!(TXS_PER_BLOCK as u64));
        assert_eq!(status["closed"], json!(false));
    }
}

/// Submits all prepared requests through the replay bypass and verifies their hashes.
/// Hash equality proves that transport serialization did not change the signed transaction.
async fn submit_replay_transactions(node: &MadaraCmd, tx_hashes: &[Felt], invoke_txs: Vec<Value>) {
    for (index, invoke_tx) in invoke_txs.into_iter().enumerate() {
        let got_hash = bypass_add_invoke_transaction(node, invoke_tx).await.unwrap();
        assert_eq!(got_hash, tx_hashes[index], "tx hash mismatch at index {index}");
    }
}

/// Waits for each replay boundary to close and validates all progress counters.
/// The last executed hash must equal the boundary anchor with no recorded mismatch.
async fn assert_replay_boundaries_closed(node: &MadaraCmd, start_block_n: u64, tx_hashes: &[Felt]) {
    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let expected_last_tx_hash = tx_hashes[(block_idx + 1) * TXS_PER_BLOCK - 1];
        let status = wait_for_cond(
            || async {
                let Some(status) = get_replay_boundary_status(node, block_n).await? else {
                    bail!("Replay boundary status missing for block #{block_n}");
                };
                if !status["closed"].as_bool().unwrap_or(false) || !status["boundary_met"].as_bool().unwrap_or(false) {
                    bail!("Boundary not closed/met yet for block #{block_n}: {status}");
                }
                Ok(status)
            },
            Duration::from_millis(250),
            600,
        )
        .await;
        assert_eq!(status["block_n"], json!(block_n));
        assert_eq!(status["expected_tx_count"], json!(TXS_PER_BLOCK as u64));
        assert_eq!(status["executed_tx_count"], json!(TXS_PER_BLOCK as u64));
        assert_eq!(status["dispatched_tx_count"], json!(TXS_PER_BLOCK as u64));
        assert_eq!(status["reached_last_tx_hash"], json!(true));
        assert_eq!(status["boundary_met"], json!(true));
        assert_eq!(status["closed"], json!(true));
        assert!(status["mismatch"].is_null());
        assert_eq!(status["last_executed_tx_hash"], json!(format!("{expected_last_tx_hash:#x}")));
    }
}

/// Waits until the final replay block is visible as the confirmed chain head.
/// This separates eventual close completion from per-boundary metadata checks.
async fn wait_for_replay_head(node: &MadaraCmd, start_block_n: u64) {
    let expected = start_block_n + BLOCK_COUNT as u64 - 1;
    wait_for_cond(
        || async {
            let latest = node.json_rpc().block_hash_and_number().await?;
            if latest.block_number < expected {
                bail!("Waiting for latest block to reach {expected} (got {})", latest.block_number);
            }
            Ok(())
        },
        Duration::from_millis(250),
        600,
    )
    .await;
}

/// Verifies every transaction receipt points at its replay-assigned block.
/// It then checks confirmed block contents preserve the exact ten-transaction slices and order.
async fn assert_replay_chain(node: &MadaraCmd, start_block_n: u64, tx_hashes: &[Felt]) {
    for (index, tx_hash) in tx_hashes.iter().enumerate() {
        let expected_block_n = start_block_n + (index / TXS_PER_BLOCK) as u64;
        let receipt = wait_for_cond(
            || async {
                let receipt = node.json_rpc().get_transaction_receipt(*tx_hash).await?;
                anyhow::ensure!(receipt.block.is_block());
                Ok(receipt)
            },
            Duration::from_millis(250),
            600,
        )
        .await;
        assert_eq!(receipt.block.block_number(), expected_block_n);
    }
    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let block = node.json_rpc().get_block_with_tx_hashes(BlockId::Number(block_n)).await.unwrap();
        let MaybePreConfirmedBlockWithTxHashes::Block(block) = block else {
            panic!("block #{block_n} should be confirmed");
        };
        let expected = &tx_hashes[block_idx * TXS_PER_BLOCK..(block_idx + 1) * TXS_PER_BLOCK];
        assert_eq!(block.transactions.len(), TXS_PER_BLOCK);
        assert_eq!(block.transactions, expected);
    }
}

#[tokio::test]
#[rstest]
async fn replay_mode_boundary_happy_path_200_txs_10_per_block() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let node = start_replay_node().await;
    let (start_block_n, tx_hashes, invoke_txs) = prepare_replay_transactions(&node).await;
    seed_replay_boundaries(&node, start_block_n, &tx_hashes).await;
    submit_replay_transactions(&node, &tx_hashes, invoke_txs).await;
    assert_replay_boundaries_closed(&node, start_block_n, &tx_hashes).await;
    wait_for_replay_head(&node, start_block_n).await;
    assert_replay_chain(&node, start_block_n, &tx_hashes).await;
}
