//! Utility functions and structures for Starknet interaction and testing.
//!
//! This module provides tools for:
//! - Managing Madara nodes for testing
//! - Creating and managing Starknet accounts
//! - Deploying contracts
//! - Sending state updates and transactions
//! - Handling test contexts and cleanup

use crate::state_update::StateUpdate;
use assert_matches::assert_matches;
use m_cairo_test_contracts::{APPCHAIN_CONTRACT_SIERRA, MESSAGING_CONTRACT_SIERRA};
use starknet_accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{
    BlockId, BlockTag, Call, ExecuteInvocation, ExecutionResult, TransactionReceipt, TransactionReceiptWithBlockInfo,
    TransactionTrace,
};
use starknet_core::utils::get_selector_from_name;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider, ProviderError};
use starknet_signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;
use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

/// Deployer Starknet account address used for testing
pub const DEPLOYER_ADDRESS: &str = "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
/// Private key for the deployer account used in tests
pub const DEPLOYER_PRIVATE_KEY: &str = "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";
/// Universal Deployer Contract address on Starknet
pub const UDC_ADDRESS: &str = "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";
/// Path to the Madara configuration file for devnet setup
pub const MADARA_CONFIG_PATH: &str = "../../../../configs/presets/devnet.yaml";

/// Type alias for a Starknet account used in tests
pub type StarknetAccount = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;
/// Type alias for transaction receipt results with error handling
pub type TransactionReceiptResult = Result<TransactionReceiptWithBlockInfo, ProviderError>;

/// Test context containing necessary components for Starknet testing
pub struct TestContext {
    pub cmd: mc_e2e_tests::MadaraCmd,
    /// Account for interacting with Starknet
    pub account: StarknetAccount,
    pub deployed_appchain_contract_address: Felt,
    pub deployed_messaging_contract_address: Felt,
}

pub async fn get_test_context() -> TestContext {
    let cmd = mc_e2e_tests::MadaraCmdBuilder::new()
        .args([
            "--devnet",
            "--chain-config-path",
            MADARA_CONFIG_PATH,
            "--no-l1-sync",
            "--chain-config-override=block_time=5s,pending_block_update_time=1s",
        ])
        .label("settlement_layer")
        .run();

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(Felt::from_str(DEPLOYER_PRIVATE_KEY).unwrap()));
    let mut account = SingleOwnerAccount::new(
        cmd.json_rpc(),
        signer,
        Felt::from_str(DEPLOYER_ADDRESS).unwrap(),
        cmd.json_rpc().chain_id().await.unwrap(),
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let (deployed_appchain_contract_address, deployed_messaging_contract_address) = (
        deploy_contract(&account, APPCHAIN_CONTRACT_SIERRA).await,
        deploy_contract(&account, MESSAGING_CONTRACT_SIERRA).await,
    );

    TestContext { cmd, account, deployed_appchain_contract_address, deployed_messaging_contract_address }
}

/// Sends a state update to the appchain contract
///
/// # Arguments
/// * `account` - The Starknet account to use for the transaction
/// * `appchain_contract_address` - The address of the appchain contract
/// * `update` - The state update to send
///
/// # Returns
/// A Result containing the block number where the update was included
///
/// # Errors
/// Returns an error if the transaction fails
pub async fn send_state_update(
    account: &StarknetAccount,
    appchain_contract_address: Felt,
    update: StateUpdate,
) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("update_state")?,
            calldata: vec![Felt::from(update.block_number.unwrap_or(0)), update.global_root, update.block_hash],
        }])
        .send()
        .await?;
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await?;

    let latest_block_number_recorded = account.provider().block_number().await?;

    match receipt.block.block_number() {
        Some(block_number) => Ok(block_number),
        None => Ok(latest_block_number_recorded + 1),
    }
}

/// Get the message hash as calculated within cairo.
pub async fn get_message_hash_from_cairo(account: &StarknetAccount, appchain_contract_address: Felt) -> Felt {
    let TransactionTrace::Invoke(call) = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("get_l1_to_l2_msg_hash").unwrap(),
            calldata: vec![],
        }])
        .simulate(true, true)
        .await
        .unwrap()
        .transaction_trace
    else {
        unreachable!();
    };

    let ExecuteInvocation::Success(call) = call.execute_invocation else {
        unreachable!("{:?}", call.execute_invocation)
    };

    call.result[0]
}

/// Fires a messaging event in the messaging contract
///
/// # Arguments
/// * `account` - The Starknet account to use for the transaction
/// * `appchain_contract_address` - The address of the messaging contract
///
/// # Returns
/// A Result containing the block number where the event was included
///
/// # Errors
/// Returns an error if the transaction fails
pub async fn fire_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> u64 {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("fire_event").unwrap(),
            calldata: vec![],
        }])
        .send()
        .await
        .unwrap();
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await.unwrap();
    assert_eq!(receipt.receipt.execution_result(), &ExecutionResult::Succeeded);

    let latest_block_number_recorded = account.provider().block_number().await.unwrap();

    match receipt.block.block_number() {
        Some(block_number) => block_number,
        None => latest_block_number_recorded + 1,
    }
}

/// Cancels a messaging event in the messaging contract
///
/// # Arguments
/// * `account` - The Starknet account to use for the transaction
/// * `appchain_contract_address` - The address of the messaging contract
///
/// # Returns
/// A Result containing the block number where the cancellation was included
///
/// # Errors
/// Returns an error if the transaction fails
pub async fn cancel_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> u64 {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("cancel_event").unwrap(),
            calldata: vec![],
        }])
        .send()
        .await
        .unwrap();
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await.unwrap();

    let latest_block_number_recorded = account.provider().block_number().await.unwrap();

    match receipt.block.block_number() {
        Some(block_number) => block_number,
        None => latest_block_number_recorded + 1,
    }
}

/// Deploys a contract to Starknet
///
/// # Arguments
/// * `account` - The Starknet account to use for deployment
/// * `sierra` - The Sierra code of the contract
/// * `casm_hash` - The CASM hash of the contract
///
/// # Returns
/// A Result containing the address of the deployed contract
///
/// # Errors
/// Returns an error if deployment fails
pub async fn deploy_contract(account: &StarknetAccount, sierra: &[u8]) -> Felt {
    let contract_artifact: SierraClass = serde_json::from_slice(sierra).unwrap();
    let flattened_class = contract_artifact.flatten().unwrap();

    let (compiled_class_hash, _compiled_class) =
        mp_class::FlattenedSierraClass::from(flattened_class.clone()).compile_to_casm().unwrap();

    let result = account.declare_v2(Arc::new(flattened_class), compiled_class_hash).send().await.unwrap();
    // wait one block
    let start = account.provider().block_number().await.unwrap();
    while account.provider().block_number().await.unwrap() <= start {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let deployment = account
        .execute_v3(vec![Call {
            to: Felt::from_str(UDC_ADDRESS).unwrap(),
            selector: get_selector_from_name("deployContract").unwrap(),
            calldata: vec![result.class_hash, Felt::ZERO, Felt::ZERO, Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();
    let deployed_contract_address =
        get_deployed_contract_address(deployment.transaction_hash, account.provider()).await.unwrap();
    // wait one block
    let start = account.provider().block_number().await.unwrap();
    while account.provider().block_number().await.unwrap() <= start {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    deployed_contract_address
}

/// Extracts the deployed contract address from a transaction receipt
///
/// # Arguments
/// * `txn_hash` - The transaction hash of the deployment transaction
/// * `provider` - The Starknet provider
///
/// # Returns
/// A Result containing the address of the deployed contract
///
/// # Errors
/// Returns an error if the address cannot be extracted
pub async fn get_deployed_contract_address(
    txn_hash: Felt,
    provider: &JsonRpcClient<HttpTransport>,
) -> anyhow::Result<Felt> {
    let deploy_tx_receipt = get_transaction_receipt(provider, txn_hash).await?;
    let contract_address = assert_matches!(
        deploy_tx_receipt,
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => {
            receipt.events.iter().find(|e| e.keys[0] == get_selector_from_name("ContractDeployed").unwrap()).unwrap().data[0]
        }
    );
    Ok(contract_address)
}

/// Gets a transaction receipt with retry logic
///
/// Polls for the transaction receipt until it is available or the max poll count is reached.
///
/// # Arguments
/// * `rpc` - The Starknet RPC client
/// * `transaction_hash` - The transaction hash to get the receipt for
///
/// # Returns
/// A TransactionReceiptResult containing the receipt or an error
pub async fn get_transaction_receipt(
    rpc: &JsonRpcClient<HttpTransport>,
    transaction_hash: Felt,
) -> TransactionReceiptResult {
    // there is a delay between the transaction being available at the client
    // and the pending tick of the block, hence sleeping for 500ms
    assert_poll(|| async { rpc.get_transaction_receipt(transaction_hash).await.is_ok() }, 500, 20).await;
    rpc.get_transaction_receipt(transaction_hash).await
}

/// Polls a condition until it returns true or the max poll count is reached
///
/// # Arguments
/// * `f` - A function that returns a Future resolving to a boolean
/// * `polling_time_ms` - Time in milliseconds between polls
/// * `max_poll_count` - Maximum number of times to poll
///
/// # Panics
/// Panics if the max poll count is reached without the condition returning true
pub async fn assert_poll<F, Fut>(f: F, polling_time_ms: u64, max_poll_count: u32)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _poll_count in 0..max_poll_count {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(polling_time_ms)).await;
    }
    panic!("Max poll count exceeded.");
}
