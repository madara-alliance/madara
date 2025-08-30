use crate::services::helpers::get_file_path;
use crate::services::helpers::NodeRpcMethods;
use crate::services::helpers::TransactionFinalityStatus;
use crate::setup::ChainSetup;
use alloy::primitives::Address;
use starknet::providers::Provider;
use starknet_core::types::BlockId;
use starknet_core::types::BlockTag;
use starknet_core::types::FunctionCall;
use starknet_core::utils::get_selector_from_name;
use starknet_signers::LocalWallet;
use std::time::Duration;
use tokio::time::sleep;
use tokio::time::Instant;

use starknet::{
    accounts::SingleOwnerAccount,
    core::types::Felt,
    providers::jsonrpc::{HttpTransport, JsonRpcClient},
};

// Constants - Taken from: addresses.json, bootstrapper.json and output of bootstrapper
pub const L2_ACCOUNT_ADDRESS: &str = "0x4fe5eea46caa0a1f344fafce82b39d66b552f00d3cd12e89073ef4b4ab37860";
pub const L2_ACCOUNT_PRIVATE_KEY: &str = "0xabcd"; // Hex Madara Account Private Key

pub const L2_ERC20_TOKEN_ADDRESS: &str = "0x25205e11d1c0017f94a531a139b47137dae34ae0b9bed9e8fe698ace64f0609"; // Hex Madara ERC20 TOKEN Address
pub const L2_ERC20_BRIDGE_ADDRESS: &str = "0x7e46129030dcff37062dd4353a738a7a5d5a88e8481bfa8b36a2ef5f8f7fa47"; // Hex Madara ERC20 BRIDGE Address

pub const L2_ETH_TOKEN_ADDRESS: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"; // Hex Madara ETH TOKEN Address
pub const L2_ETH_BRIDGE_ADDRESS: &str = "0x190f2407f7040ef9a60d4df4d2eace6089419aa9ec42cda229a82a29b2d5b3e"; // Hex Madara ETH BRIDGE Address

pub const L1_ACCOUNT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"; // Hex L1 Account Address
pub const L1_ACCOUNT_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"; // Hex L1 Account Private Key

pub const L1_ERC20_BRIDGE_ADDRESS: &str = "0x59b670e9fa9d0a427751af201d676719a970857b"; // Hex L1 ERC20 BRIDGE Address
pub const L1_ERC20_TOKEN_ADDRESS: &str = "0x4ed7c70f96b99c776995fb64377f0d4ab3b0e1c1";

pub const L1_ETH_BRIDGE_ADDRESS: &str = "0x8a791620dd6260079bf849dc5567adc3f2fdc318"; // Hex L1 ETH BRIDGE Address

pub type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Helper structures for better organization - using type inference approach

pub struct L1Context {
    pub eth_bridge_address: Address,
    pub erc20_token_address: Address,
    pub erc20_bridge_address: Address,
}

pub struct L2Context {
    pub account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    pub address: Felt,
    pub eth_token_address: Felt,
    pub eth_bridge_address: Felt,
    pub erc20_token_address: Felt,
    pub erc20_bridge_address: Felt,
}

pub async fn wait_for_transactions_finality(setup: &ChainSetup, transaction_hashes: Vec<Felt>) -> TestResult<()> {
    let madara_service = setup.lifecycle_manager.madara_service().as_ref().ok_or("Madara service not available")?;

    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(500);
    let polling_interval = Duration::from_secs(12);

    println!("⏳ Waiting for {} transactions to reach finality...", transaction_hashes.len());

    // Keep track of which txs are still pending
    let mut pending_txs: Vec<Felt> = transaction_hashes.clone();

    loop {
        if start_time.elapsed() >= timeout_duration {
            return Err(format!(
                "Transaction finality check timed out after 500 seconds. Still pending: {:?}",
                pending_txs.iter().map(|f| f.to_hex_string()).collect::<Vec<_>>()
            )
            .into());
        }

        println!(
            "Checking finality for {} pending transactions... (elapsed: {:?})",
            pending_txs.len(),
            start_time.elapsed()
        );

        let mut newly_finalized = Vec::new();

        for tx_hash in &pending_txs {
            match madara_service.get_transaction_finality(tx_hash.to_hex_string().as_str()).await {
                Ok(txn_finality) => {
                    println!("Transaction {} status: {:?}", tx_hash.to_hex_string(), txn_finality);

                    if txn_finality == TransactionFinalityStatus::AcceptedOnL1 {
                        println!(
                            "✅ Transaction {} finalized after {:?}",
                            tx_hash.to_hex_string(),
                            start_time.elapsed()
                        );
                        newly_finalized.push(*tx_hash);
                    }
                }
                Err(e) => {
                    println!("Error checking finality for {}: {:?}", tx_hash.to_hex_string(), e);
                }
            }
        }

        // Remove finalized transactions from the pending list
        pending_txs.retain(|tx| !newly_finalized.contains(tx));

        // If all transactions are finalized, we’re done
        if pending_txs.is_empty() {
            println!("🎉 All transactions finalized in {:?}", start_time.elapsed());
            break;
        }

        println!(
            "⏳ {} transactions still pending, waiting {} seconds before next check...",
            pending_txs.len(),
            polling_interval.as_secs()
        );

        sleep(polling_interval).await;
    }

    Ok(())
}

pub async fn get_l2_token_balance(
    provider: &JsonRpcClient<HttpTransport>,
    contract_address: Felt,
    account_address: Felt,
) -> TestResult<Felt> {
    let result = provider
        .call(
            FunctionCall {
                contract_address,
                entry_point_selector: get_selector_from_name("balanceOf")
                    .map_err(|e| format!("Failed to get balanceOf selector: {}", e))?,
                calldata: vec![account_address],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .map_err(|e| format!("Failed to call balanceOf: {}", e))?;

    result.first().copied().ok_or("balanceOf returned empty result".into())
}

pub fn cleanup_test_directory(test_name: &str) {
    let dir_path = get_file_path(test_name);
    match std::fs::remove_dir_all(&dir_path) {
        Ok(_) => println!("🗑️  Test directory cleaned up successfully"),
        Err(err) => eprintln!("⚠️  Failed to delete directory: {}", err),
    }
}

// Keep the original helper function for backward compatibility
pub async fn l2_read_token_balance(
    rpc: &JsonRpcClient<HttpTransport>,
    contract_address: Felt,
    account_address: Felt,
) -> Vec<Felt> {
    rpc.call(
        FunctionCall {
            contract_address,
            entry_point_selector: get_selector_from_name("balanceOf").unwrap(),
            calldata: vec![account_address],
        },
        BlockId::Tag(BlockTag::Latest),
    )
    .await
    .unwrap()
}
