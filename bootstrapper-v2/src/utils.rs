use std::future::Future;

use anyhow::Context;
use starknet::{
    core::types::{ExecutionResult, Felt, TransactionReceipt, TransactionReceiptWithBlockInfo},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError},
};

pub fn save_addresses_to_file(addresses_json: String, file_path: &str) -> anyhow::Result<()> {
    // Ensure parent directories exist before writing the file
    if let Some(parent_dir) = std::path::Path::new(&file_path).parent() {
        std::fs::create_dir_all(parent_dir)
            .with_context(|| format!("Failed to create parent directories for: {}", &file_path))?;
    }

    std::fs::write(file_path, &addresses_json)
        .with_context(|| format!("Failed to write addresses to: {}", file_path))?;

    Ok(())
}

pub async fn assert_poll<F, Fut>(f: F, polling_time_ms: u64, max_poll_count: u32)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _poll_count in 0..max_poll_count {
        if f().await {
            return; // The provided function returned true, exit safely.
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(polling_time_ms)).await;
    }

    panic!("Max poll count exceeded.");
}

pub async fn get_transaction_receipt(
    rpc: &JsonRpcClient<HttpTransport>,
    transaction_hash: Felt,
) -> TransactionReceiptResult {
    // there is a delay between the transaction being available at the client
    // and the sealing of the block, hence sleeping for 500ms
    assert_poll(|| async { rpc.get_transaction_receipt(transaction_hash).await.is_ok() }, 500, 20).await;

    rpc.get_transaction_receipt(transaction_hash).await
}

// Creating this type for better readability
type TransactionReceiptResult = Result<TransactionReceiptWithBlockInfo, ProviderError>;

pub async fn wait_for_transaction(
    provider_l2: &JsonRpcClient<HttpTransport>,
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
