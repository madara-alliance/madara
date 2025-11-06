use crate::contract_clients::config::RpcClientProvider;
use crate::contract_clients::utils::RpcAccount;
use crate::helpers::account_actions::{get_transaction_receipt, AccountActions};
use ethers::abi::Address;
use ethers::types::U256;
use num_bigint::BigUint;
use serde_json::{Map, Value};
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{ExecutionResult, Felt, InvokeTransactionResult, TransactionReceipt};
use std::path::Path;
use std::str::FromStr;
use std::{fs, io};
use std::thread::sleep;
use std::time::Duration;

pub mod banner;
pub mod constants;

pub async fn invoke_contract(
    contract: Felt,
    method: &str,
    calldata: Vec<Felt>,
    account: &RpcAccount<'_>,
) -> InvokeTransactionResult {
    let txn_res =
        account.invoke_contract(contract, method, calldata, None).send().await.expect("Error in invoking the contract");

    wait_for_transaction(
        account.provider(),
        txn_res.transaction_hash,
        &format!("invoking_contract with method {:?}", method),
    )
    .await
    .unwrap();

    txn_res
}

pub fn pad_bytes(address: Address) -> Vec<u8> {
    let address_bytes = address.as_bytes();
    let mut padded_address_bytes = Vec::with_capacity(32);
    padded_address_bytes.extend(vec![0u8; 32 - address_bytes.len()]);
    padded_address_bytes.extend_from_slice(address_bytes);
    padded_address_bytes
}

pub fn hexstring_to_address(hex: &str) -> ethers::abi::Address {
    let hexstring = format!("0x{:0>40}", hex.strip_prefix("0x").unwrap_or(hex));
    Address::from_str(&hexstring).expect("Hexstring to Address conversion failed")
}

pub async fn wait_for_transaction(
    provider_l2: &RpcClientProvider,
    transaction_hash: Felt,
    tag: &str,
) -> Result<(), anyhow::Error> {
    let transaction_receipt = get_transaction_receipt(provider_l2, transaction_hash).await;

    let transaction_status = transaction_receipt.ok().unwrap();

    log::trace!("txn : {:?} : {:?}", tag, transaction_status);
    let exec_result: ExecutionResult = match transaction_status.receipt {
        TransactionReceipt::Invoke(receipt) => receipt.execution_result,
        TransactionReceipt::DeployAccount(receipt) => {
            let contract_address = receipt.contract_address;
            log::info!("Account deployed at {:?}", contract_address);
            receipt.execution_result
        }
        TransactionReceipt::Declare(receipt) => receipt.execution_result,
        TransactionReceipt::Deploy(receipt) => {
            log::info!("Tag: {:?}, Contract deployed at address {:?}", tag, receipt.contract_address);
            receipt.execution_result
        }
        TransactionReceipt::L1Handler(receipt) => receipt.execution_result,
    };

    match exec_result {
        ExecutionResult::Succeeded => {}
        ExecutionResult::Reverted { reason } => {
            panic!("Transaction failed with {:?}", reason);
        }
    }

    Ok(())
}

pub fn convert_felt_to_u256(felt: Felt) -> U256 {
    U256::from_big_endian(&felt.to_bytes_be())
}

pub enum JsonValueType {
    EthAddress(Address),
    StringType(String),
}

pub fn save_to_json(key: &str, value: &JsonValueType) -> Result<(), io::Error> {
    let file_path: &str = "./data/addresses.json";
    let data = fs::read_to_string(file_path);
    let mut json: Map<String, Value> = match data {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| Map::new()),
        Err(_) => Map::new(),
    };

    match value {
        JsonValueType::EthAddress(x) => {
            json.insert(key.to_string(), serde_json::json!(x));
        }
        JsonValueType::StringType(x) => {
            json.insert(key.to_string(), serde_json::json!(convert_to_hex(x)));
        }
    }

    let updated_json = serde_json::to_string_pretty(&json)?;

    // Ensure the directory exists before writing the file
    if let Some(dir_path) = Path::new(file_path).parent() {
        fs::create_dir_all(dir_path)?;
    }

    fs::write(file_path, updated_json)?;

    Ok(())
}

pub fn convert_to_hex(address: &str) -> String {
    let big_uint = address.parse::<BigUint>().map_err(|_| "Invalid number");
    let hex = big_uint.expect("error converting decimal string ---> hex string").to_str_radix(16);
    "0x".to_string() + &hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hexstring_to_address() {
        // Test full-length address with 0x prefix
        assert_eq!(
            hexstring_to_address("0x8464135c8F25Da09e49BC8782676a84730C318bC"),
            Address::from_str("0x8464135c8F25Da09e49BC8782676a84730C318bC").unwrap()
        );

        // Test full-length address without 0x prefix
        assert_eq!(
            hexstring_to_address("8464135c8F25Da09e49BC8782676a84730C318bC"),
            Address::from_str("0x8464135c8F25Da09e49BC8782676a84730C318bC").unwrap()
        );

        // Test short address that needs padding
        assert_eq!(
            hexstring_to_address("0xabcd"),
            Address::from_str("0x000000000000000000000000000000000000abcd").unwrap()
        );

        // Test short address without 0x prefix
        assert_eq!(
            hexstring_to_address("abcd"),
            Address::from_str("0x000000000000000000000000000000000000abcd").unwrap()
        );

        // Test empty string with 0x prefix
        assert_eq!(
            hexstring_to_address("0x"),
            Address::from_str("0x0000000000000000000000000000000000000000").unwrap()
        );

        // Test empty string
        assert_eq!(hexstring_to_address(""), Address::from_str("0x0000000000000000000000000000000000000000").unwrap());
    }

    #[test]
    #[should_panic(expected = "Hexstring to Address conversion failed")]
    fn test_invalid_hex_characters() {
        hexstring_to_address("0xZZZZ"); // Invalid hex characters
    }

    #[test]
    #[should_panic(expected = "Hexstring to Address conversion failed")]
    fn test_oversized_input() {
        hexstring_to_address("0x8464135c8F25Da09e49BC8782676a84730C318bCFF"); // Too long
    }
}
