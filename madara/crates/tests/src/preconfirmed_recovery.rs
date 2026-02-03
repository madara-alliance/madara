use crate::{
    devnet::{ACCOUNT_ADDRESS, ACCOUNT_SECRET, ACCOUNT_SECRETS, ACCOUNTS, ERC20_STRK_CONTRACT_ADDRESS},
    wait_for_cond, MadaraCmd, MadaraCmdBuilder,
};
use anyhow::{bail, ensure};
use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{
    BlockId, BlockTag, Call, ExecutionResult, Felt, TransactionReceiptWithBlockInfo, TransactionStatus,
};
use starknet_core::utils::starknet_keccak;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider};
use std::time::Duration;
use std::convert::TryFrom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownKind {
    Graceful,
    Ungraceful,
}

fn devnet_args(block_time: &str, block_production_disabled: bool) -> Vec<String> {
    let mut args = vec![
        "--devnet".into(),
        "--no-l1-sync".into(),
        "--l1-gas-price".into(),
        "0".into(),
        "--blob-gas-price".into(),
        "0".into(),
        "--chain-config-override".into(),
        format!("block_time={block_time}"),
    ];

    if block_production_disabled {
        args.push("--no-block-production".into());
    }

    args
}

fn env_pairs(no_mempool_saving: bool, no_save_preconfirmed: bool) -> Vec<(String, String)> {
    vec![
        ("MADARA_NO_MEMPOOL_SAVING".to_string(), no_mempool_saving.to_string()),
        ("MADARA_NO_SAVE_PRECONFIRMED".to_string(), no_save_preconfirmed.to_string()),
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

fn transfer_call(recipient: Felt, amount_low: Felt, amount_high: Felt) -> Call {
    Call {
        to: ERC20_STRK_CONTRACT_ADDRESS,
        selector: starknet_keccak(b"transfer"),
        calldata: vec![recipient, amount_low, amount_high],
    }
}

async fn submit_success_and_revert_txs(node: &MadaraCmd) -> (Felt, Felt) {
    let account_success = build_account_for(node, ACCOUNT_ADDRESS, ACCOUNT_SECRET).await;
    let account_revert = build_account_for(node, ACCOUNTS[1], ACCOUNT_SECRETS[1]).await;

    let recipient = ACCOUNTS[2];

    let (l1_price, l2_price, l1_data_price) = fetch_gas_prices(node).await;

    let success = account_success
        .execute_v3(vec![transfer_call(recipient, Felt::ONE, Felt::ZERO)])
        .l1_gas(1_000_000)
        .l1_gas_price(l1_price)
        .l2_gas(1_000_000)
        .l2_gas_price(l2_price)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(l1_data_price)
        .tip(0)
        .send()
        .await
        .unwrap();

    // Use a very large amount (2^128) to force a revert on balance check.
    let revert = account_revert
        .execute_v3(vec![transfer_call(recipient, Felt::ZERO, Felt::ONE)])
        .l1_gas(1_000_000)
        .l1_gas_price(l1_price)
        .l2_gas(1_000_000)
        .l2_gas_price(l2_price)
        .l1_data_gas(1_000_000)
        .l1_data_gas_price(l1_data_price)
        .tip(0)
        .send()
        .await
        .unwrap();

    (success.transaction_hash, revert.transaction_hash)
}

async fn fetch_gas_prices(node: &MadaraCmd) -> (u128, u128, u128) {
    let block = node
        .json_rpc()
        .get_block_with_tx_hashes(BlockId::Tag(BlockTag::Latest))
        .await
        .unwrap();

    let l1_price = u128::try_from(block.l1_gas_price().price_in_fri).expect("l1 gas price should fit u128");
    let l2_price = u128::try_from(block.l2_gas_price().price_in_fri).expect("l2 gas price should fit u128");
    let l1_data_price =
        u128::try_from(block.l1_data_gas_price().price_in_fri).expect("l1 data gas price should fit u128");

    (l1_price, l2_price, l1_data_price)
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

async fn wait_for_status_received(node: &MadaraCmd, tx_hash: Felt) {
    wait_for_cond(
        || async {
            let status = node.json_rpc().get_transaction_status(tx_hash).await?;
            ensure!(matches!(status, TransactionStatus::Received), "status={status:?}");
            Ok(())
        },
        Duration::from_millis(500),
        120,
    )
    .await;
}

async fn assert_status_not_found(node: &MadaraCmd, tx_hash: Felt) {
    for _ in 0..20 {
        if node.json_rpc().get_transaction_status(tx_hash).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("expected tx {tx_hash:#x} to be missing, but it is still reported");
}

async fn wait_for_received_or_accepted(node: &MadaraCmd, tx_hash: Felt) {
    wait_for_cond(
        || async {
            if let Ok(receipt) = node.json_rpc().get_transaction_receipt(tx_hash).await {
                if receipt.block.is_block() || receipt.block.is_pre_confirmed() {
                    return Ok(());
                }
            }

            let status = node.json_rpc().get_transaction_status(tx_hash).await?;
            match status {
                TransactionStatus::Received | TransactionStatus::AcceptedOnL2(_) => Ok(()),
                _ => bail!("status={status:?}"),
            }
        },
        Duration::from_millis(500),
        120,
    )
    .await;
}

fn shutdown(node: &mut MadaraCmd, kind: ShutdownKind) {
    match kind {
        ShutdownKind::Graceful => node.stop(),
        ShutdownKind::Ungraceful => node.kill(),
    }
}

/// Preconfirmed-recovery test (executed path).
///
/// What this test does:
/// - Starts a devnet node with block production enabled and a long block time.
/// - Submits two txs (one succeeds, one reverts) from two different pre-funded accounts.
/// - Waits for both txs to be executed into the preconfirmed block.
/// - Shuts down the node (graceful or ungraceful), restarts with the same DB.
///
/// Expected behavior:
/// - Graceful shutdown: preconfirmed block is closed → both txs are confirmed after restart.
/// - Ungraceful + save_preconfirmed=true: preconfirmed block is recovered/re-executed → confirmed.
/// - Ungraceful + save_preconfirmed=false:
///   - If mempool saving is ON, txs may reappear as Received or quickly become AcceptedOnL2.
///   - If mempool saving is OFF, txs should be missing.
///
/// This test runs across the 4 env combinations:
/// - MADARA_NO_MEMPOOL_SAVING {false,true} × MADARA_NO_SAVE_PRECONFIRMED {false,true}
#[rstest]
// persist mempool + persist preconfirmed, graceful → behaviour: Madara closes preconfirmed block on shutdown -> Expected: txs status confirmed after restart
#[case::e00_graceful(false, false, ShutdownKind::Graceful)]
// no mempool persistence + persist preconfirmed, graceful → behaviour: Madara closes preconfirmed block on shutdown -> Expected: txs status confirmed after restart
#[case::e10_graceful(true, false, ShutdownKind::Graceful)]
// persist mempool + no preconfirmed persistence, graceful → behaviour: Madara closes preconfirmed block on shutdown -> Expected: txs status confirmed after restart
#[case::e01_graceful(false, true, ShutdownKind::Graceful)]
// no mempool persistence + no preconfirmed persistence, graceful → behaviour: Madara closes preconfirmed block on shutdown -> Expected: txs status confirmed after restart
#[case::e11_graceful(true, true, ShutdownKind::Graceful)]
// persist mempool + persist preconfirmed, ungraceful → behaviour: preconfirmed persisted & re-executed on startup -> Expected: txs re-executed after restart and confirmed
#[case::e00_ungraceful(false, false, ShutdownKind::Ungraceful)]
// no mempool persistence + persist preconfirmed, ungraceful → behaviour: preconfirmed persisted & re-executed on startup -> Expected: txs re-executed after restart and confirmed + no "Could not add txns from mempool log"
#[case::e10_ungraceful(true, false, ShutdownKind::Ungraceful)]
// persist mempool + no preconfirmed persistence, ungraceful → behaviour: preconfirmed lost, mempool should restore -> Expected: txs Received or AcceptedOnL2 after restart
#[case::e01_ungraceful(false, true, ShutdownKind::Ungraceful)]
// no mempool persistence + no preconfirmed persistence, ungraceful → behaviour: preconfirmed lost, mempool not persisted -> Expected: txs missing after restart
#[case::e11_ungraceful(true, true, ShutdownKind::Ungraceful)]
#[tokio::test]
async fn preconfirmed_recovery_executed(
    #[case] no_mempool_saving: bool,
    #[case] no_save_preconfirmed: bool,
    #[case] shutdown_kind: ShutdownKind,
) {
    let args = devnet_args("5min", false);
    let builder = MadaraCmdBuilder::new().args(args).env(env_pairs(no_mempool_saving, no_save_preconfirmed));

    let mut node = start_node(builder.clone()).await;

    let (tx_success, tx_revert) = submit_success_and_revert_txs(&node).await;

    let receipt_success = wait_for_preconfirmed_receipt(&node, tx_success).await;
    assert_eq!(receipt_success.receipt.execution_result(), &ExecutionResult::Succeeded);

    let receipt_revert = wait_for_preconfirmed_receipt(&node, tx_revert).await;
    assert!(matches!(receipt_revert.receipt.execution_result(), ExecutionResult::Reverted { .. }));

    shutdown(&mut node, shutdown_kind);
    drop(node);

    let mut node = start_node(builder).await;

    let expect_confirmed = shutdown_kind == ShutdownKind::Graceful || !no_save_preconfirmed;
    if expect_confirmed {
        let receipt_success = wait_for_confirmed_receipt(&node, tx_success).await;
        assert_eq!(receipt_success.receipt.execution_result(), &ExecutionResult::Succeeded);

        let receipt_revert = wait_for_confirmed_receipt(&node, tx_revert).await;
        assert!(matches!(receipt_revert.receipt.execution_result(), ExecutionResult::Reverted { .. }));
    } else if !no_mempool_saving {
        // Preconfirmed not saved, but mempool persistence may re-introduce txs as Received
        // or the txs may get executed/confirmed quickly on restart.
        wait_for_received_or_accepted(&node, tx_success).await;
        wait_for_received_or_accepted(&node, tx_revert).await;
    } else {
        assert_status_not_found(&node, tx_success).await;
        assert_status_not_found(&node, tx_revert).await;
    }

    shutdown(&mut node, ShutdownKind::Graceful);
}

/// Preconfirmed-recovery test (mempool-only path).
///
/// What this test does:
/// - Starts a devnet node with block production disabled.
/// - Submits two txs (one succeeds, one reverts) from two different accounts.
/// - Verifies both are in mempool (status Received), then shuts down (graceful or ungraceful).
/// - Restarts with the same DB.
///
/// Expected behavior:
/// - With mempool saving ON: txs should reappear as Received after restart.
/// - With mempool saving OFF: txs should be missing after restart.
///
/// Note: MADARA_NO_SAVE_PRECONFIRMED is a no-op here (no preconfirmed block exists), so we fix it
/// to false and do not matrix it.
#[rstest]
// persist mempool (MADARA_NO_SAVE_PRECONFIRMED ignored), graceful → behaviour: mempool persisted -> Expected: txs Received after restart
#[case::e00_graceful(false, ShutdownKind::Graceful)]
// no mempool persistence (MADARA_NO_SAVE_PRECONFIRMED ignored), graceful → behaviour: mempool not persisted -> Expected: txs missing after restart
#[case::e10_graceful(true, ShutdownKind::Graceful)]
// persist mempool (MADARA_NO_SAVE_PRECONFIRMED ignored), ungraceful → behaviour: mempool persisted -> Expected: txs Received after restart
#[case::e00_ungraceful(false, ShutdownKind::Ungraceful)]
// no mempool persistence (MADARA_NO_SAVE_PRECONFIRMED ignored), ungraceful → behaviour: mempool not persisted -> Expected: txs missing after restart
#[case::e10_ungraceful(true, ShutdownKind::Ungraceful)]
#[tokio::test]
async fn preconfirmed_recovery_mempool(
    #[case] no_mempool_saving: bool,
    #[case] shutdown_kind: ShutdownKind,
) {
    let args = devnet_args("5min", true);
    // No preconfirmed block exists in this test, so MADARA_NO_SAVE_PRECONFIRMED is fixed to false.
    let builder = MadaraCmdBuilder::new().args(args).env(env_pairs(no_mempool_saving, false));

    let mut node = start_node(builder.clone()).await;

    let (tx_success, tx_revert) = submit_success_and_revert_txs(&node).await;

    wait_for_status_received(&node, tx_success).await;
    wait_for_status_received(&node, tx_revert).await;

    shutdown(&mut node, shutdown_kind);
    drop(node);

    let mut node = start_node(builder).await;

    if no_mempool_saving {
        assert_status_not_found(&node, tx_success).await;
        assert_status_not_found(&node, tx_revert).await;
    } else {
        wait_for_status_received(&node, tx_success).await;
        wait_for_status_received(&node, tx_revert).await;
    }

    shutdown(&mut node, ShutdownKind::Graceful);
}
