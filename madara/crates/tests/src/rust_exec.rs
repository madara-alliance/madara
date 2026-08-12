use crate::devnet::{ACCOUNTS, ACCOUNT_ADDRESS, ACCOUNT_SECRET, ACCOUNT_SECRETS, ERC20_STRK_CONTRACT_ADDRESS};
use crate::{wait_for_cond, MadaraCmdBuilder};
use anyhow::ensure;
use mc_devnet::RUST_EXEC_TRANSFER_CONTRACT_ADDRESS;
use serde_json::json;
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, ExecutionResult, Felt, FunctionCall};
use starknet_core::utils::starknet_keccak;
use starknet_providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet_providers::Provider;
use std::time::Duration;
use url::Url;

fn json_rpc_v0_10_0(node: &crate::MadaraCmd) -> JsonRpcClient<HttpTransport> {
    let endpoint = format!("{}/rpc/v0_10_0/", node.rpc_url().trim_end_matches('/'));
    JsonRpcClient::new(HttpTransport::new(Url::parse(&endpoint).expect("valid Madara RPC URL")))
}

async fn admin_rpc(admin_url: &str, method: &str) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(admin_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": [],
        }))
        .send()
        .await
        .expect("admin RPC request should be sent")
        .error_for_status()
        .expect("admin RPC HTTP response should succeed")
        .json::<serde_json::Value>()
        .await
        .expect("admin RPC response should be JSON");
    assert!(response.get("error").is_none(), "{method} failed: {response}");
    response.get("result").cloned().expect("admin RPC response should contain a result")
}

async fn close_block(admin_url: &str) {
    admin_rpc(admin_url, "madara_closeBlock").await;
}

#[tokio::test]
async fn devnet_routes_only_supported_transfer_through_rust_exec() {
    let executor_addresses = format!("{ACCOUNT_ADDRESS:#x},{:#x}", ACCOUNTS[1]);
    let mut node = MadaraCmdBuilder::new()
        .capture_logs()
        .args([
            "--devnet".to_string(),
            "--no-l1-sync".to_string(),
            "--no-charge-fee".to_string(),
            "--chain-config-path".to_string(),
            "test_devnet.yaml".to_string(),
            "--chain-config-override".to_string(),
            "block_time=30s".to_string(),
            "--startup-execution-mode".to_string(),
            "mixed".to_string(),
            "--executor-addresses".to_string(),
            executor_addresses,
            "--rpc-admin".to_string(),
            "--rpc-unsafe".to_string(),
        ])
        .run();
    node.wait_for_ready().await;

    let provider = json_rpc_v0_10_0(&node);
    let chain_id = provider.chain_id().await.expect("devnet chain id");
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account = SingleOwnerAccount::new(provider, signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    let rust_signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRETS[1]));
    let mut rust_account =
        SingleOwnerAccount::new(json_rpc_v0_10_0(&node), rust_signer, ACCOUNTS[1], chain_id, ExecutionEncoding::New);
    rust_account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    let log_cursor = node.log_cursor();

    let normal_transfer = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![crate::ACCOUNTS[1], Felt::ONE, Felt::ZERO],
        }])
        .send()
        .await
        .expect("ordinary ERC20 transfer should be accepted");
    wait_for_cond(
        || async {
            let logs = node.logs_since(log_cursor);
            ensure!(logs.iter().any(|line| line.contains("Executed and added 1 transaction")));
            Ok(())
        },
        Duration::from_millis(100),
        100,
    )
    .await;
    assert!(
        !node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_rust_exec")),
        "ordinary ERC20 transfer unexpectedly used Rust Exec"
    );

    let rust_cursor = node.log_cursor();
    let rust_transfer = rust_account
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNTS[2], Felt::from(42u64)],
        }])
        .send()
        .await
        .expect("Rust Exec transfer should be accepted");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(rust_cursor).iter().any(|line| line.contains("executed_with_rust_exec")));
            Ok(())
        },
        Duration::from_millis(100),
        100,
    )
    .await;

    close_block(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            let receipt = rust_account.provider().get_transaction_receipt(rust_transfer.transaction_hash).await?;
            ensure!(!receipt.block.is_pre_confirmed());
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    let rust_receipt = rust_account
        .provider()
        .get_transaction_receipt(rust_transfer.transaction_hash)
        .await
        .expect("Rust transfer receipt should be confirmed");
    assert_eq!(rust_receipt.receipt.execution_result(), &ExecutionResult::Succeeded);
    let normal_receipt = account
        .provider()
        .get_transaction_receipt(normal_transfer.transaction_hash)
        .await
        .expect("ordinary transfer receipt should be confirmed");
    assert_eq!(normal_receipt.receipt.execution_result(), &ExecutionResult::Succeeded);

    let transfer_state = account
        .provider()
        .call(
            &FunctionCall {
                contract_address: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                entry_point_selector: starknet_keccak(b"get_last_transfer"),
                calldata: vec![],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .expect("transfer fixture state should be readable");
    assert_eq!(transfer_state, vec![ACCOUNTS[1], ACCOUNTS[2], Felt::from(42u64), Felt::ONE]);

    let logs = node.logs_since(log_cursor);
    assert_eq!(logs.iter().filter(|line| line.contains("executed_with_rust_exec")).count(), 1);
    assert!(
        logs.iter().any(|line| line.contains("comparator_passed")),
        "comparator did not accept the mixed block:\n{}",
        logs.join("\n")
    );
}

#[tokio::test]
async fn devnet_comparator_mismatch_promotes_blockifier_and_disables_rust_exec() {
    let executor_address = format!("{ACCOUNT_ADDRESS:#x}");
    let mut node = MadaraCmdBuilder::new()
        .capture_logs()
        .args([
            "--devnet".to_string(),
            "--no-l1-sync".to_string(),
            "--no-charge-fee".to_string(),
            "--chain-config-path".to_string(),
            "test_devnet.yaml".to_string(),
            "--chain-config-override".to_string(),
            "block_time=30s".to_string(),
            "--startup-execution-mode".to_string(),
            "mixed".to_string(),
            "--executor-addresses".to_string(),
            executor_address,
            "--rpc-admin".to_string(),
            "--rpc-unsafe".to_string(),
        ])
        .run();
    node.wait_for_ready().await;

    let provider = json_rpc_v0_10_0(&node);
    let chain_id = provider.chain_id().await.expect("devnet chain id");
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account = SingleOwnerAccount::new(provider, signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    let mismatch_amount = Felt::from(77u64);

    let mismatch_cursor = node.log_cursor();
    let mismatch_tx = account
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer_with_comparator_mismatch"),
            calldata: vec![crate::ACCOUNTS[1], mismatch_amount],
        }])
        .send()
        .await
        .expect("mismatch fixture transaction should be accepted");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(mismatch_cursor).iter().any(|line| line.contains("executed_with_rust_exec")));
            Ok(())
        },
        Duration::from_millis(100),
        100,
    )
    .await;

    close_block(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            let status = admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await;
            ensure!(status["mode"] == "blockifier_only");
            ensure!(status["reason"] == "state_diff_mismatch");
            ensure!(status["replay_backlog_empty"] == true);
            ensure!(status["comparator_enabled"] == false);
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    let mismatch_receipt = account
        .provider()
        .get_transaction_receipt(mismatch_tx.transaction_hash)
        .await
        .expect("mismatch transaction receipt should be confirmed");
    assert_eq!(mismatch_receipt.receipt.execution_result(), &ExecutionResult::Succeeded);

    let canonical_state = account
        .provider()
        .call(
            &FunctionCall {
                contract_address: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                entry_point_selector: starknet_keccak(b"get_last_transfer"),
                calldata: vec![],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .expect("canonical fallback state should be readable");
    assert_eq!(
        canonical_state,
        vec![ACCOUNT_ADDRESS, crate::ACCOUNTS[1], mismatch_amount, Felt::ONE],
        "the mismatching block must close with Blockifier's Cairo state"
    );

    let fallback_cursor = node.log_cursor();
    let fallback_amount = Felt::from(88u64);
    let fallback_tx = account
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer_with_comparator_mismatch"),
            calldata: vec![crate::ACCOUNTS[1], fallback_amount],
        }])
        .nonce(Felt::ONE)
        .send()
        .await
        .expect("post-fallback transaction should be accepted");
    wait_for_cond(
        || async {
            ensure!(node
                .logs_since(fallback_cursor)
                .iter()
                .any(|line| line.contains("Executed and added 1 transaction")));
            Ok(())
        },
        Duration::from_millis(100),
        100,
    )
    .await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            let receipt = account.provider().get_transaction_receipt(fallback_tx.transaction_hash).await?;
            ensure!(!receipt.block.is_pre_confirmed());
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;

    let fallback_state = account
        .provider()
        .call(
            &FunctionCall {
                contract_address: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                entry_point_selector: starknet_keccak(b"get_last_transfer"),
                calldata: vec![],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .expect("post-fallback state should be readable");
    assert_eq!(
        fallback_state,
        vec![ACCOUNT_ADDRESS, crate::ACCOUNTS[1], fallback_amount, Felt::from(2u64)],
        "sticky Blockifier-only execution must use the Cairo value"
    );

    let fallback_logs = node.logs_since(fallback_cursor);
    assert!(
        !fallback_logs.iter().any(|line| line.contains("executed_with_rust_exec")),
        "Rust Exec was used after strict fallback:\n{}",
        fallback_logs.join("\n")
    );
}
