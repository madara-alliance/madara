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

async fn flush_mempool(admin_url: &str) {
    admin_rpc(admin_url, "madara_flushMempool").await;
}

fn devnet_account(
    node: &crate::MadaraCmd,
    index: usize,
    chain_id: Felt,
) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRETS[index]));
    let mut account =
        SingleOwnerAccount::new(json_rpc_v0_10_0(node), signer, ACCOUNTS[index], chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    account
}

fn mixed_node_builder(executor_addresses: &[Felt], extra_args: &[&str]) -> MadaraCmdBuilder {
    let executor_addresses =
        executor_addresses.iter().map(|address| format!("{address:#x}")).collect::<Vec<_>>().join(",");
    let mut args = vec![
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
    ];
    args.extend(extra_args.iter().map(|arg| (*arg).to_string()));
    MadaraCmdBuilder::new().capture_logs().env([("RUST_LOG", "debug")]).args(args)
}

async fn start_mixed_node(executor_addresses: &[Felt], extra_args: &[&str]) -> (crate::MadaraCmd, Felt) {
    let mut node = mixed_node_builder(executor_addresses, extra_args).run();
    node.wait_for_ready().await;
    let chain_id = json_rpc_v0_10_0(&node).chain_id().await.expect("devnet chain id");
    (node, chain_id)
}

fn rust_transfer(recipient: Felt, amount: Felt) -> Call {
    Call {
        to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount],
    }
}

async fn wait_for_rust_executions(node: &crate::MadaraCmd, cursor: usize, expected: usize) {
    wait_for_cond(
        || async {
            let count = node.logs_since(cursor).iter().filter(|line| line.contains("executed_with_rust_exec")).count();
            ensure!(count >= expected, "expected {expected} Rust executions, observed {count}");
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
}

async fn wait_for_confirmed_success(provider: &JsonRpcClient<HttpTransport>, transaction_hash: Felt) {
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(transaction_hash).await?;
            ensure!(!receipt.block.is_pre_confirmed());
            ensure!(receipt.receipt.execution_result() == &ExecutionResult::Succeeded);
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
}

async fn last_transfer(provider: &JsonRpcClient<HttpTransport>) -> Vec<Felt> {
    provider
        .call(
            &FunctionCall {
                contract_address: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                entry_point_selector: starknet_keccak(b"get_last_transfer"),
                calldata: vec![],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .expect("transfer fixture state should be readable")
}

#[tokio::test]
async fn devnet_routes_only_supported_transfer_through_rust_exec() {
    let executor_addresses = format!("{ACCOUNT_ADDRESS:#x},{:#x}", ACCOUNTS[1]);
    let mut node = MadaraCmdBuilder::new()
        .capture_logs()
        .env([("RUST_LOG", "debug")])
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

#[rstest::rstest]
#[case::optimistic("optimistic")]
#[case::sequential("sequential")]
#[tokio::test]
async fn devnet_batch_barrier_resets_for_the_next_logical_batch(#[case] pipeline_mode: &str) {
    let executor_addresses = format!("{:#x},{:#x},{:#x}", ACCOUNTS[0], ACCOUNTS[1], ACCOUNTS[2]);
    let mut node = MadaraCmdBuilder::new()
        .capture_logs()
        .env([("RUST_LOG", "debug")])
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
            "--batch-size".to_string(),
            "2".to_string(),
            "--mempool-paused".to_string(),
            "--block-pipeline-mode".to_string(),
            pipeline_mode.to_string(),
            "--rpc-admin".to_string(),
            "--rpc-unsafe".to_string(),
        ])
        .run();
    node.wait_for_ready().await;

    let chain_id = json_rpc_v0_10_0(&node).chain_id().await.expect("devnet chain id");
    let account_0 = devnet_account(&node, 0, chain_id);
    let account_1 = devnet_account(&node, 1, chain_id);
    let account_2 = devnet_account(&node, 2, chain_id);
    let log_cursor = node.log_cursor();

    let rust_first = account_0
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNTS[3], Felt::from(41u64)],
        }])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("first Rust transaction should enter the paused mempool");
    let cairo_suffix = account_1
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNTS[3], Felt::ONE, Felt::ZERO],
        }])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("Cairo transaction should enter the paused mempool");
    let rust_next_batch = account_2
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNTS[4], Felt::from(43u64)],
        }])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("next-batch Rust transaction should enter the paused mempool");

    flush_mempool(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            let logs = node.logs_since(log_cursor);
            ensure!(logs.iter().filter(|line| line.contains("batch_routed")).count() >= 2);
            ensure!(logs.iter().filter(|line| line.contains("batch_executed")).count() >= 2);
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;

    let logs = node.logs_since(log_cursor);
    let rust_indices = logs
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains("executed_with_rust_exec").then_some(index))
        .collect::<Vec<_>>();
    let cairo_index = logs
        .iter()
        .position(|line| line.contains("executed_with_blockifier"))
        .expect("the first logical batch must execute a Cairo suffix");
    assert!(rust_indices.len() >= 2, "both logical batches must execute a Rust prefix:\n{}", logs.join("\n"));
    assert!(
        rust_indices[0] < cairo_index && cairo_index < rust_indices[1],
        "execution must be Rust prefix, Cairo suffix, then a fresh Rust batch:\n{}",
        logs.join("\n")
    );

    close_block(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            for hash in [rust_first.transaction_hash, cairo_suffix.transaction_hash, rust_next_batch.transaction_hash] {
                let receipt = account_0.provider().get_transaction_receipt(hash).await?;
                ensure!(!receipt.block.is_pre_confirmed());
                ensure!(receipt.receipt.execution_result() == &ExecutionResult::Succeeded);
            }
            let status = admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await;
            ensure!(status["mode"] == "mixed");
            ensure!(status["comparator_enabled"] == true);
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;

    let transfer_state = account_0
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
        .expect("final transfer fixture state should be readable");
    assert_eq!(transfer_state, vec![ACCOUNTS[2], ACCOUNTS[4], Felt::from(43u64), Felt::from(2u64)]);
    assert!(
        node.logs_since(log_cursor).iter().any(|line| line.contains("comparator_passed")),
        "comparator must accept the source-ordered mixed block"
    );
}

#[tokio::test]
async fn devnet_comparator_mismatch_promotes_blockifier_and_disables_rust_exec() {
    let executor_address = format!("{ACCOUNT_ADDRESS:#x}");
    let mut node = MadaraCmdBuilder::new()
        .capture_logs()
        .env([("RUST_LOG", "debug")])
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

#[tokio::test]
async fn devnet_preserves_unread_zero_write_against_confirmed_parent() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);

    let seed_cursor = node.log_cursor();
    let seed = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::from(55u64))])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("nonzero seed transfer should be accepted");
    wait_for_rust_executions(&node, seed_cursor, 1).await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, seed.transaction_hash).await;
    assert_eq!(last_transfer(&provider).await, vec![ACCOUNT_ADDRESS, ACCOUNTS[1], Felt::from(55u64), Felt::ONE]);

    let clear_cursor = node.log_cursor();
    let clear = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::ZERO)])
        .nonce(Felt::ONE)
        .send()
        .await
        .expect("zero transfer should be accepted");
    wait_for_rust_executions(&node, clear_cursor, 1).await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, clear.transaction_hash).await;

    assert_eq!(
        last_transfer(&provider).await,
        vec![ACCOUNT_ADDRESS, ACCOUNTS[1], Felt::ZERO, Felt::TWO],
        "an unread Rust write of zero must clear the nonzero parent value"
    );
    let logs = node.logs_since(clear_cursor);
    assert!(logs.iter().any(|line| line.contains("comparator_passed")), "zero-write block was not accepted");
    assert!(!logs.iter().any(|line| line.contains("comparator_failed")), "zero-write block mismatched");
    let status = admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await;
    assert_eq!(status["mode"], "mixed");
}

#[tokio::test]
async fn devnet_zero_write_then_later_overwrite_preserves_logical_batch_order() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &["--mempool-paused", "--batch-size", "1"]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);
    let log_cursor = node.log_cursor();

    let first = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::from(7u64))])
        .nonce(Felt::ZERO)
        .l1_gas(1_000_000)
        .l1_gas_price(1)
        .l2_gas(2_000_000)
        .l2_gas_price(1)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(1)
        .send()
        .await
        .expect("first transfer should enter the paused mempool");
    let clear = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[2], Felt::ZERO)])
        .nonce(Felt::ONE)
        .l1_gas(1_000_000)
        .l1_gas_price(1)
        .l2_gas(2_000_000)
        .l2_gas_price(1)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(1)
        .send()
        .await
        .expect("zero transfer should enter the paused mempool");
    let overwrite = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[3], Felt::from(9u64))])
        .nonce(Felt::TWO)
        .l1_gas(1_000_000)
        .l1_gas_price(1)
        .l2_gas(2_000_000)
        .l2_gas_price(1)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(1)
        .send()
        .await
        .expect("later overwrite should enter the paused mempool");

    flush_mempool(&node.rpc_admin_url()).await;
    wait_for_rust_executions(&node, log_cursor, 3).await;
    close_block(&node.rpc_admin_url()).await;
    for hash in [first.transaction_hash, clear.transaction_hash, overwrite.transaction_hash] {
        wait_for_confirmed_success(&provider, hash).await;
    }

    assert_eq!(
        last_transfer(&provider).await,
        vec![ACCOUNT_ADDRESS, ACCOUNTS[3], Felt::from(9u64), Felt::from(3u64)],
        "the later transaction must win without hiding the intermediate zero write from comparison"
    );
    let logs = node.logs_since(log_cursor);
    assert!(logs.iter().filter(|line| line.contains("batch_routed")).count() >= 3);
    assert!(logs.iter().any(|line| line.contains("comparator_passed")));
    assert!(!logs.iter().any(|line| line.contains("comparator_failed")));
}

#[tokio::test]
async fn devnet_supported_multicall_observes_prior_call_writes() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);
    let log_cursor = node.log_cursor();

    let tx = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::from(11u64)), rust_transfer(ACCOUNTS[2], Felt::from(22u64))])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("supported multicall should be accepted");
    wait_for_rust_executions(&node, log_cursor, 1).await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, tx.transaction_hash).await;

    assert_eq!(
        last_transfer(&provider).await,
        vec![ACCOUNT_ADDRESS, ACCOUNTS[2], Felt::from(22u64), Felt::TWO],
        "the second Rust call must read the first call's transfer_count write"
    );
    let logs = node.logs_since(log_cursor);
    assert!(logs.iter().any(|line| line.contains("comparator_passed")));
    assert!(!logs.iter().any(|line| line.contains("comparator_failed")));
}

#[tokio::test]
async fn devnet_partially_supported_multicall_fails_closed_to_blockifier() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);
    let log_cursor = node.log_cursor();

    let tx = account
        .execute_v3(vec![
            rust_transfer(ACCOUNTS[1], Felt::from(31u64)),
            Call {
                to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                selector: starknet_keccak(b"get_last_transfer"),
                calldata: vec![],
            },
        ])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("partially supported multicall should be accepted by Blockifier");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_blockifier")));
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    assert!(
        !node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_rust_exec")),
        "a partially supported multicall must fail closed as one Blockifier transaction"
    );

    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, tx.transaction_hash).await;
    assert_eq!(last_transfer(&provider).await, vec![ACCOUNT_ADDRESS, ACCOUNTS[1], Felt::from(31u64), Felt::ONE]);
    assert_eq!(admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await["mode"], "mixed");
}

#[tokio::test]
async fn devnet_unlisted_executor_fails_closed_to_blockifier() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 1, chain_id);
    let provider = json_rpc_v0_10_0(&node);
    let log_cursor = node.log_cursor();

    let tx = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[2], Felt::from(41u64))])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("unlisted executor transaction should be accepted by Blockifier");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_blockifier")));
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    assert!(
        !node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_rust_exec")),
        "an unlisted account must not route through Rust Exec"
    );

    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, tx.transaction_hash).await;
    assert_eq!(last_transfer(&provider).await, vec![ACCOUNTS[1], ACCOUNTS[2], Felt::from(41u64), Felt::ONE]);
}

#[tokio::test]
async fn devnet_rust_runtime_failure_reexecutes_with_blockifier_and_stays_safe() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);
    let log_cursor = node.log_cursor();

    let failed = account
        .execute_v3(vec![Call {
            to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNTS[1]],
        }])
        .nonce(Felt::ZERO)
        .l1_gas(1_000_000)
        .l1_gas_price(1)
        .l2_gas(2_000_000)
        .l2_gas_price(1)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(1)
        .send()
        .await
        .expect("malformed supported call should enter the block");

    wait_for_cond(
        || async {
            let status = admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await;
            ensure!(status["mode"] == "blockifier_only");
            ensure!(status["reason"] == "rust_runtime_failure");
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_cond(
        || async {
            let receipt = provider.get_transaction_receipt(failed.transaction_hash).await?;
            ensure!(!receipt.block.is_pre_confirmed());
            ensure!(matches!(receipt.receipt.execution_result(), ExecutionResult::Reverted { .. }));
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;

    let fallback_cursor = node.log_cursor();
    let valid = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[2], Felt::from(42u64))])
        .nonce(Felt::ONE)
        .send()
        .await
        .expect("post-fallback transaction should be accepted");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(fallback_cursor).iter().any(|line| line.contains("executed_with_blockifier")));
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    assert!(!node.logs_since(fallback_cursor).iter().any(|line| line.contains("executed_with_rust_exec")));
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, valid.transaction_hash).await;
    assert_eq!(last_transfer(&provider).await, vec![ACCOUNT_ADDRESS, ACCOUNTS[2], Felt::from(42u64), Felt::ONE]);
    assert!(node.logs_since(log_cursor).iter().any(|line| line.contains("executed_with_blockifier")));
}

#[tokio::test]
async fn devnet_manual_disable_then_enable_switches_engines_at_block_boundaries() {
    let (node, chain_id) = start_mixed_node(&[ACCOUNT_ADDRESS], &[]).await;
    let account = devnet_account(&node, 0, chain_id);
    let provider = json_rpc_v0_10_0(&node);

    admin_rpc(&node.rpc_admin_url(), "madara_executionboxDisable").await;
    assert_eq!(admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await["mode"], "blockifier_only");
    let disabled_cursor = node.log_cursor();
    let disabled_tx = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::from(51u64))])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("disabled-mode transaction should be accepted");
    wait_for_cond(
        || async {
            ensure!(node.logs_since(disabled_cursor).iter().any(|line| line.contains("executed_with_blockifier")));
            Ok(())
        },
        Duration::from_millis(100),
        200,
    )
    .await;
    assert!(!node.logs_since(disabled_cursor).iter().any(|line| line.contains("executed_with_rust_exec")));
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, disabled_tx.transaction_hash).await;

    admin_rpc(&node.rpc_admin_url(), "madara_executionboxEnable").await;
    wait_for_cond(
        || async {
            ensure!(admin_rpc(&node.rpc_admin_url(), "madara_executionboxStatus").await["mode"] == "mixed");
            Ok(())
        },
        Duration::from_millis(100),
        100,
    )
    .await;
    let enabled_cursor = node.log_cursor();
    let enabled_tx = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[2], Felt::from(52u64))])
        .nonce(Felt::ONE)
        .send()
        .await
        .expect("re-enabled transaction should be accepted");
    wait_for_rust_executions(&node, enabled_cursor, 1).await;
    close_block(&node.rpc_admin_url()).await;
    wait_for_confirmed_success(&provider, enabled_tx.transaction_hash).await;

    assert_eq!(last_transfer(&provider).await, vec![ACCOUNT_ADDRESS, ACCOUNTS[2], Felt::from(52u64), Felt::TWO]);
    assert!(node.logs_since(enabled_cursor).iter().any(|line| line.contains("comparator_passed")));
}

#[tokio::test]
async fn devnet_restart_recovers_unconfirmed_rust_block_with_blockifier() {
    let builder = mixed_node_builder(&[ACCOUNT_ADDRESS], &[]);
    let mut node = builder.clone().run();
    node.wait_for_ready().await;
    let chain_id = json_rpc_v0_10_0(&node).chain_id().await.expect("devnet chain id");
    let account = devnet_account(&node, 0, chain_id);
    let log_cursor = node.log_cursor();

    let tx = account
        .execute_v3(vec![rust_transfer(ACCOUNTS[1], Felt::from(61u64))])
        .nonce(Felt::ZERO)
        .send()
        .await
        .expect("Rust transaction should enter the preconfirmed block");
    wait_for_rust_executions(&node, log_cursor, 1).await;
    node.kill();
    drop(node);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut restarted = builder.run();
    restarted.wait_for_ready().await;
    let provider = json_rpc_v0_10_0(&restarted);
    wait_for_confirmed_success(&provider, tx.transaction_hash).await;
    assert_eq!(
        last_transfer(&provider).await,
        vec![ACCOUNT_ADDRESS, ACCOUNTS[1], Felt::from(61u64), Felt::ONE],
        "startup recovery must rebuild the persisted mixed block from Blockifier output"
    );
    assert!(
        !restarted.logs_since(0).iter().any(|line| line.contains("executed_with_rust_exec")),
        "startup recovery must not trust and rerun speculative Rust output"
    );
}
