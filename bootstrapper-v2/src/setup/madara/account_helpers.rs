use anyhow::anyhow;
use starknet::core::types::{TransactionReceipt, TransactionReceiptWithBlockInfo};

use starknet::{
    core::types::{Felt, InvokeTransactionResult},
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
};

use crate::utils::{get_transaction_receipt, wait_for_transaction};

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

/// Extracts the addresses from the ContractsDeployed event from MadaraFactory
pub async fn get_contracts_deployed_addresses(
    rpc: &JsonRpcClient<HttpTransport>,
    tx: &InvokeTransactionResult,
) -> anyhow::Result<ContractsDeployedAddresses> {
    let tx_hash = tx.transaction_hash;

    wait_for_transaction(rpc, tx_hash, "get_contracts_deployed_addresses").await.unwrap();

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
