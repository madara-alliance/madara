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

#[tokio::test]
#[rstest]
async fn replay_mode_boundary_happy_path_200_txs_10_per_block() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

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

        let tx_hash = prepared.transaction_hash(false);
        let invoke_tx = prepared.get_invoke_request(false, false).await.unwrap();

        tx_hashes.push(tx_hash);
        invoke_txs.push(serde_json::to_value(invoke_tx).unwrap());
        next_nonce += Felt::ONE;
    }

    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let last_tx_hash = tx_hashes[(block_idx + 1) * TXS_PER_BLOCK - 1];
        let status = set_replay_boundary(&node, block_n, TXS_PER_BLOCK as u64, last_tx_hash).await.unwrap();
        assert_eq!(status["block_n"], json!(block_n));
        assert_eq!(status["expected_tx_count"], json!(TXS_PER_BLOCK as u64));
        assert_eq!(status["closed"], json!(false));
    }

    for (idx, invoke_tx) in invoke_txs.into_iter().enumerate() {
        let got_hash = bypass_add_invoke_transaction(&node, invoke_tx).await.unwrap();
        assert_eq!(got_hash, tx_hashes[idx], "tx hash mismatch at index {idx}");
    }

    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let expected_last_tx_hash = tx_hashes[(block_idx + 1) * TXS_PER_BLOCK - 1];

        let status = wait_for_cond(
            || async {
                let Some(status) = get_replay_boundary_status(&node, block_n).await? else {
                    bail!("Replay boundary status missing for block #{block_n}");
                };

                let closed = status["closed"].as_bool().unwrap_or(false);
                let boundary_met = status["boundary_met"].as_bool().unwrap_or(false);
                if !closed || !boundary_met {
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
        assert!(status["mismatch"].is_null(), "unexpected mismatch for block #{block_n}: {status}");
        assert_eq!(
            status["last_executed_tx_hash"],
            json!(format!("{expected_last_tx_hash:#x}")),
            "last executed tx hash mismatch for block #{block_n}"
        );
    }

    wait_for_cond(
        || async {
            let latest = node.json_rpc().block_hash_and_number().await?;
            if latest.block_number < start_block_n + BLOCK_COUNT as u64 - 1 {
                bail!(
                    "Waiting for latest block to reach {} (got {})",
                    start_block_n + BLOCK_COUNT as u64 - 1,
                    latest.block_number
                );
            }
            Ok(())
        },
        Duration::from_millis(250),
        600,
    )
    .await;

    for (idx, tx_hash) in tx_hashes.iter().enumerate() {
        let expected_block_n = start_block_n + (idx / TXS_PER_BLOCK) as u64;
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
        assert_eq!(receipt.block.block_number(), expected_block_n, "tx #{idx} landed in unexpected block");
    }

    for block_idx in 0..BLOCK_COUNT {
        let block_n = start_block_n + block_idx as u64;
        let block = node.json_rpc().get_block_with_tx_hashes(BlockId::Number(block_n)).await.unwrap();
        let MaybePreConfirmedBlockWithTxHashes::Block(block) = block else {
            panic!("block #{block_n} should be confirmed");
        };

        let expected_hashes = &tx_hashes[block_idx * TXS_PER_BLOCK..(block_idx + 1) * TXS_PER_BLOCK];
        assert_eq!(block.transactions.len(), TXS_PER_BLOCK, "unexpected tx count in block #{block_n}");
        assert_eq!(block.transactions, expected_hashes, "transaction order/content mismatch in block #{block_n}");
    }
}
