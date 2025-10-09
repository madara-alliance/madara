use std::{future::Future, sync::Arc};

use anyhow::Context;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{
        contract::{CompiledClass, SierraClass},
        BlockId, BlockTag, Call, InvokeTransactionResult,
    },
    signers::LocalWallet,
};

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

    let transaction_status = transaction_receipt.context("Failed to get transaction receipt")?;

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

pub async fn declare_contract(
    sierra_path: &str,
    casm_path: &str,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> anyhow::Result<Felt> {
    log::info!("sierra_path: {:?}", sierra_path);
    log::info!("casm_path: {:?}", casm_path);
    let contract_artifact: SierraClass = serde_json::from_reader(
        std::fs::File::open(sierra_path).context("Failed to open sierra file")?
    ).context("Failed to parse sierra file")?;

    let contract_artifact_casm: CompiledClass =
        serde_json::from_reader(
            std::fs::File::open(casm_path).context("Failed to open casm file")?
        ).context("Failed to parse casm file")?;

    let class_hash = contract_artifact.class_hash().context("Failed to get class hash from sierra artifact")?;
    let compiled_class_hash = contract_artifact_casm.class_hash().context("Failed to get class hash from casm artifact")?;

    if account.provider().get_class(BlockId::Tag(BlockTag::Pending), class_hash).await.is_ok() {
        log::info!("Class already declared, skipping declaration.");
        return Ok(class_hash);
    }

    let flattened_class = contract_artifact.flatten().context("Failed to flatten contract artifact")?;

    let txn = account
        .declare_v3(Arc::new(flattened_class), compiled_class_hash)
        .gas(0)
        .send()
        .await
        .expect("Error in declaring the contract using Cairo 1 declaration using the provided account");
    wait_for_transaction(account.provider(), txn.transaction_hash, "declare_contract").await.context("Failed to wait for contract declaration transaction")?;
    Ok(class_hash)
}

pub async fn execute_v3(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    calls: &Vec<Call>,
) -> anyhow::Result<InvokeTransactionResult, anyhow::Error> {
    let txn_res = account
        .execute_v3(calls.clone())
        .gas(0)
        .send()
        .await
        .context("Error in making execute_v3 the contract for calls {:?}")?;

    wait_for_transaction(
        account.provider(),
        txn_res.transaction_hash,
        &format!("invoking_contract for calls {:?}", calls),
    )
    .await
    .context("Failed to wait for transaction execution")?;

    Ok(txn_res)
}
