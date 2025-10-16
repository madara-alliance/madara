use std::{future::Future, sync::Arc};

use anyhow::{anyhow, Context};
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
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, ProviderError},
};

use std::io::Error as IoError;

use crate::error::madara::MadaraError;

#[derive(thiserror::Error, Debug)]
pub enum FileError {
    #[error("Failed to create parent directories with path: {0} due to: {1}")]
    FailedCreatingParentDirectory(String, #[source] IoError),

    #[error("Failed to write to file: {0}")]
    FailedToWriteFile(#[source] IoError),
}

pub fn save_addresses_to_file(addresses_json: String, file_path: &str) -> Result<(), FileError> {
    // Ensure parent directories exist before writing the file
    if let Some(parent_dir) = std::path::Path::new(&file_path).parent() {
        std::fs::create_dir_all(parent_dir)
            .map_err(|e| FileError::FailedCreatingParentDirectory(file_path.to_string(), e))?;
    }

    std::fs::write(file_path, &addresses_json).map_err(|e| FileError::FailedToWriteFile(e))?;

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
) -> Result<(), MadaraError> {
    let transaction_receipt = get_transaction_receipt(provider_l2, transaction_hash).await;

    let transaction_status = transaction_receipt?;

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
        ExecutionResult::Succeeded => Ok(()),
        ExecutionResult::Reverted { reason } => {
            return Err(MadaraError::FailedToWaitForTransaction(reason, tag.to_string()));
        }
    }
}

pub async fn declare_contract(
    sierra_path: &str,
    casm_path: &str,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
) -> Result<Felt, MadaraError> {
    log::info!("Declaring contract from sierra file: {:?}", sierra_path);
    log::info!("Declaring contract from casm file: {:?}", casm_path);
    let contract_artifact: SierraClass = serde_json::from_reader(
        std::fs::File::open(sierra_path).map_err(|e| MadaraError::FailedToOpenFile(e, sierra_path.to_string()))?,
    )
    .map_err(|e| MadaraError::FailedToParseFile(e, sierra_path.to_string()))?;

    let contract_artifact_casm: CompiledClass = serde_json::from_reader(
        std::fs::File::open(casm_path).map_err(|e| MadaraError::FailedToOpenFile(e, casm_path.to_string()))?,
    )
    .map_err(|e| MadaraError::FailedToParseFile(e, casm_path.to_string()))?;

    let class_hash = contract_artifact.class_hash()?;
    let compiled_class_hash = contract_artifact_casm.class_hash()?;

    if account.provider().get_class(BlockId::Tag(BlockTag::PreConfirmed), class_hash).await.is_ok() {
        log::info!("Class already declared, skipping declaration.");
        return Ok(class_hash);
    }

    let flattened_class = contract_artifact.flatten()?;

    let txn = account
        .declare_v3(Arc::new(flattened_class), compiled_class_hash)
        .l1_gas(0)
        .l2_gas(0)
        .l1_data_gas(0)
        .send()
        .await?;
    wait_for_transaction(account.provider(), txn.transaction_hash, "declare_contract")
        .await
        .context("Failed to wait for contract declaration transaction")?;
    Ok(class_hash)
}

pub async fn execute_v3(
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    calls: &Vec<Call>,
) -> Result<InvokeTransactionResult, MadaraError> {
    let txn_res = account.execute_v3(calls.clone()).l1_gas(0).l2_gas(0).l1_data_gas(0).send().await?;

    wait_for_transaction(
        account.provider(),
        txn_res.transaction_hash,
        &format!("invoking_contract for calls {:?}", calls),
    )
    .await?;

    Ok(txn_res)
}

/// Represents the addresses from the ContractsDeployed event
#[derive(Debug, Clone)]
pub struct ContractsDeployedAddresses {
    pub l2_eth_token: Felt,
    pub l2_eth_bridge: Felt,
    pub l2_token_bridge: Felt,
}

pub async fn get_contract_address_from_deploy_tx(
    rpc: &JsonRpcClient<HttpTransport>,
    tx: &InvokeTransactionResult,
) -> Result<Felt, MadaraError> {
    let deploy_tx_hash = tx.transaction_hash;

    wait_for_transaction(rpc, deploy_tx_hash, "get_contract_address_from_deploy_tx").await?;

    let deploy_tx_receipt = get_transaction_receipt(rpc, deploy_tx_hash).await?;

    let contract_address = match deploy_tx_receipt {
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => {
            receipt
                .events
                .iter()
                .find(|e| e.keys[0] == get_selector_from_name("ContractDeployed").unwrap())
                .ok_or(MadaraError::FailedToGetEventFromTransactionReceipt("ContractDeployed".to_string()))?
                .data[0]
        }
        _ => return Err(MadaraError::ExpectedInvokeTransactionReceipt),
    };
    Ok(contract_address)
}

/// Extracts the addresses from the ContractsDeployed event from MadaraFactory
pub async fn get_contracts_deployed_addresses(
    rpc: &JsonRpcClient<HttpTransport>,
    tx: &InvokeTransactionResult,
) -> anyhow::Result<ContractsDeployedAddresses> {
    let tx_hash = tx.transaction_hash;

    wait_for_transaction(rpc, tx_hash, "get_contracts_deployed_addresses").await?;

    let tx_receipt = get_transaction_receipt(rpc, tx_hash).await?;

    let contracts_deployed_event = match tx_receipt {
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => receipt
            .events
            .iter()
            .find(|e| e.keys[0] == get_selector_from_name("ContractsDeployed").unwrap())
            .ok_or_else(|| anyhow!("ContractsDeployed event not found"))?
            .clone(),
        _ => return Err(anyhow!("Expected invoke transaction receipt")),
    };

    // The event data contains 3 addresses in order: l2_eth_token, l2_eth_bridge, l2_token_bridge
    if contracts_deployed_event.data.len() < 3 {
        return Err(anyhow!("ContractsDeployed event data too short, expected 3 addresses"));
    }

    let addresses = ContractsDeployedAddresses {
        l2_eth_token: contracts_deployed_event.data[0],
        l2_eth_bridge: contracts_deployed_event.data[1],
        l2_token_bridge: contracts_deployed_event.data[2],
    };

    Ok(addresses)
}
