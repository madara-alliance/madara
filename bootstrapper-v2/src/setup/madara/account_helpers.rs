use anyhow::anyhow;
use starknet::core::types::{TransactionReceipt, TransactionReceiptWithBlockInfo};

use starknet::{
    core::types::{Felt, InvokeTransactionResult},
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
};

use crate::utils::{get_transaction_receipt, wait_for_transaction};

pub async fn get_contract_address_from_deploy_tx(
    rpc: &JsonRpcClient<HttpTransport>,
    tx: &InvokeTransactionResult,
) -> anyhow::Result<Felt> {
    let deploy_tx_hash = tx.transaction_hash;

    wait_for_transaction(rpc, deploy_tx_hash, "get_contract_address_from_deploy_tx").await.unwrap();

    let deploy_tx_receipt = get_transaction_receipt(rpc, deploy_tx_hash).await?;

    let contract_address = match deploy_tx_receipt {
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => {
            receipt
                .events
                .iter()
                .find(|e| e.keys[0] == get_selector_from_name("ContractDeployed").unwrap())
                .unwrap()
                .data[0]
        }
        _ => return Err(anyhow!("Expected invoke transaction receipt")),
    };
    Ok(contract_address)
}
