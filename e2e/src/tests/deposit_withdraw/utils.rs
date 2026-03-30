use crate::services::constants::{
    BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT, BOOTSTRAPPER_V2_MADARA_ADDRESSES_OUTPUT, DATA_DIR,
};
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

// Constants that are fixed (Anvil defaults / Madara devnet key)
pub const L2_ACCOUNT_PRIVATE_KEY: &str = "0xabcd"; // Hex Madara Account Private Key
pub const L1_ACCOUNT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
pub const L1_ACCOUNT_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

pub type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Addresses deployed by bootstrapper v2, loaded from output JSON files
pub struct DeployedAddresses {
    // L2 addresses (from madara_addresses.json)
    pub l2_account_address: String,
    pub l2_eth_token_address: String,
    pub l2_eth_bridge_address: String,
    pub l2_erc20_token_address: String,
    pub l2_erc20_bridge_address: String,
    // L1 addresses (from base_addresses.json)
    pub l1_eth_bridge_address: String,
    pub l1_erc20_token_address: String,
    pub l1_erc20_bridge_address: String,
}

impl DeployedAddresses {
    /// Load addresses from bootstrapper v2 output files in the given test directory
    pub fn load(test_name: &str) -> TestResult<Self> {
        let base_path = get_file_path(&format!("{}/{}", test_name, BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT));
        let madara_path = get_file_path(&format!("{}/{}", test_name, BOOTSTRAPPER_V2_MADARA_ADDRESSES_OUTPUT));

        // Try test directory first, fall back to DATA_DIR
        let base_content = std::fs::read_to_string(&base_path)
            .or_else(|_| {
                let fallback = get_file_path(&format!("{}/{}", DATA_DIR, BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT));
                std::fs::read_to_string(fallback)
            })
            .map_err(|e| format!("Failed to read base_addresses.json: {}", e))?;

        let madara_content = std::fs::read_to_string(&madara_path)
            .or_else(|_| {
                let fallback = get_file_path(&format!("{}/{}", DATA_DIR, BOOTSTRAPPER_V2_MADARA_ADDRESSES_OUTPUT));
                std::fs::read_to_string(fallback)
            })
            .map_err(|e| format!("Failed to read madara_addresses.json: {}", e))?;

        let base: serde_json::Value =
            serde_json::from_str(&base_content).map_err(|e| format!("Failed to parse base_addresses.json: {}", e))?;
        let madara: serde_json::Value = serde_json::from_str(&madara_content)
            .map_err(|e| format!("Failed to parse madara_addresses.json: {}", e))?;

        let get_addr = |json: &serde_json::Value, key: &str| -> TestResult<String> {
            json["addresses"][key]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("Address '{}' not found in JSON", key).into())
        };

        // L2 account address is deterministic based on class hash + salt + deployer
        // Read it from madara_addresses.json if available, otherwise it's computed by bootstrapper
        let l2_account_address =
            madara["addresses"]["l2_account"].as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                // Fallback: this is the deterministic address from bootstrapper v2
                // with private key 0xabcd and salt 0x626f6f7473747261705f73616c74
                "0x4da71bd1d9153651a8e393bdbee29e2cafc9bade4cf17a5d56501ff764e4c78".to_string()
            });

        Ok(Self {
            l2_account_address,
            l2_eth_token_address: get_addr(&madara, "l2_eth_token")?,
            l2_eth_bridge_address: get_addr(&madara, "l2_eth_bridge")?,
            l2_erc20_token_address: get_addr(&madara, "l2_fee_token")?,
            l2_erc20_bridge_address: get_addr(&madara, "l2_token_bridge")?,
            l1_eth_bridge_address: get_addr(&base, "ethTokenBridge")?,
            l1_erc20_token_address: get_addr(&base, "l1Token")?,
            l1_erc20_bridge_address: get_addr(&base, "tokenBridge")?,
        })
    }
}

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
    let timeout_duration = Duration::from_secs(1000);
    let polling_interval = Duration::from_secs(12);

    println!("⏳ Waiting for {} transactions to reach finality...", transaction_hashes.len());

    // Keep track of which txs are still pending
    let mut pending_txs: Vec<Felt> = transaction_hashes.clone();

    loop {
        if start_time.elapsed() >= timeout_duration {
            return Err(format!(
                "Transaction finality check timed out after {} seconds. Still pending: {:?}",
                timeout_duration.as_secs(),
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

        // If all transactions are finalized, we're done
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
