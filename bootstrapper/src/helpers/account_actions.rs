use crate::contract_clients::config::RpcClientProvider;
use crate::contract_clients::utils::RpcAccount;
use crate::utils::wait_for_transaction;
use assert_matches::assert_matches;
use async_trait::async_trait;
use starknet::core::types::contract::legacy::LegacyContractClass;
use starknet::core::types::{Felt, FlattenedSierraClass, TransactionReceipt};
use starknet::core::utils::get_selector_from_name;
use starknet::providers::Provider;
use starknet::signers::LocalWallet;
use starknet::{
    accounts::{Account, SingleOwnerAccount},
    core::types::Call,
};
use starknet_accounts::ExecutionV3;
use starknet_core::types::contract::{CompiledClass, SierraClass};
use starknet_core::types::{InvokeTransactionResult, TransactionReceiptWithBlockInfo};
use starknet_core::utils::starknet_keccak;
use starknet_providers::ProviderError;
use std::future::Future;

pub type TransactionExecution<'a> = ExecutionV3<'a, RpcAccount<'a>>;

#[async_trait]
pub trait AccountActions {
    fn invoke_contract(
        &self,
        address: Felt,
        method: &str,
        calldata: Vec<Felt>,
        nonce: Option<u64>,
    ) -> TransactionExecution<'_>;

    fn declare_contract_params_sierra(&self, path_to_sierra: &str, path_to_casm: &str) -> (Felt, FlattenedSierraClass);
    fn declare_contract_params_legacy(&self, path_to_compiled_contract: &str) -> LegacyContractClass;

    fn transfer(&self, erc20_addr: Felt, receiver: Felt, amount: u128) -> TransactionExecution<'_>;
}

impl AccountActions for SingleOwnerAccount<&RpcClientProvider, LocalWallet> {
    fn invoke_contract(
        &self,
        address: Felt,
        method: &str,
        calldata: Vec<Felt>,
        nonce: Option<u64>,
    ) -> TransactionExecution<'_> {
        let calls = vec![Call { to: address, selector: get_selector_from_name(method).unwrap(), calldata }];
        match nonce {
            Some(nonce) => self.execute_v3(calls).nonce(nonce.into()),
            None => self.execute_v3(calls).l1_gas(0).l2_gas(0).l1_data_gas(0),
        }
    }

    fn declare_contract_params_sierra(&self, path_to_sierra: &str, path_to_casm: &str) -> (Felt, FlattenedSierraClass) {
        let sierra: SierraClass = serde_json::from_reader(
            std::fs::File::open(env!("CARGO_MANIFEST_DIR").to_owned() + "/" + path_to_sierra).unwrap(),
        )
        .unwrap();

        let flattened_class = sierra.flatten().unwrap();

        let casm: CompiledClass = serde_json::from_reader(
            std::fs::File::open(env!("CARGO_MANIFEST_DIR").to_owned() + "/" + path_to_casm).unwrap(),
        )
        .unwrap();

        (casm.class_hash().unwrap(), flattened_class)
    }

    fn declare_contract_params_legacy(&self, path_to_compiled_contract: &str) -> LegacyContractClass {
        let contract_artifact: LegacyContractClass = serde_json::from_reader(
            std::fs::File::open(env!("CARGO_MANIFEST_DIR").to_owned() + "/" + path_to_compiled_contract).unwrap(),
        )
        .unwrap();

        contract_artifact
    }

    fn transfer(&self, erc20_addr: Felt, receiver: Felt, amount: u128) -> TransactionExecution<'_> {
        self.execute_v3(vec![Call {
            to: erc20_addr,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![receiver, amount.into(), Felt::ZERO],
        }])
    }
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

type TransactionReceiptResult = Result<TransactionReceiptWithBlockInfo, ProviderError>;

pub async fn get_transaction_receipt(rpc: &RpcClientProvider, transaction_hash: Felt) -> TransactionReceiptResult {
    // there is a delay between the transaction being available at the client
    // and the sealing of the block, hence sleeping for 500ms
    assert_poll(|| async { rpc.get_transaction_receipt(transaction_hash).await.is_ok() }, 500, 20).await;

    rpc.get_transaction_receipt(transaction_hash).await
}

pub async fn wait_at_least_block(rpc: &RpcClientProvider, blocks_to_wait_for: Option<u64>) {
    // If options is none, we wait for 1 block
    let current_block = rpc.block_number().await.unwrap();
    let target_block = match blocks_to_wait_for {
        Some(blocks) => current_block + blocks,
        None => current_block + 1,
    };

    assert_poll(|| async { rpc.block_number().await.unwrap() >= target_block }, 500, 3 * 7200).await;
}

pub async fn get_contract_address_from_deploy_tx(
    rpc: &RpcClientProvider,
    tx: &InvokeTransactionResult,
) -> Result<Felt, ProviderError> {
    let deploy_tx_hash = tx.transaction_hash;

    wait_for_transaction(rpc, deploy_tx_hash, "get_contract_address_from_deploy_tx").await.unwrap();

    let deploy_tx_receipt = get_transaction_receipt(rpc, deploy_tx_hash).await?;

    let contract_address = assert_matches!(
        deploy_tx_receipt,
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => {
            receipt.events.iter().find(|e| e.keys[0] == get_selector_from_name("ContractDeployed").unwrap()).unwrap().data[0]
        }
    );
    Ok(contract_address)
}
