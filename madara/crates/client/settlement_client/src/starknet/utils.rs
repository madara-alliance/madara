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
use lazy_static::lazy_static;
use starknet_accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{BlockId, BlockTag, Call, TransactionReceipt, TransactionReceiptWithBlockInfo};
use starknet_core::utils::get_selector_from_name;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider, ProviderError};
use starknet_signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;
use std::future::Future;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Once;
use std::thread;
use tokio::sync::Mutex;
use url::Url;

use m_cairo_test_contracts::{APPCHAIN_CONTRACT_SIERRA, MESSAGING_CONTRACT_SIERRA};
use std::net::TcpListener;
use std::time::Duration;

/// Deployer Starknet account address used for testing
pub const DEPLOYER_ADDRESS: &str = "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
/// Private key for the deployer account used in tests
pub const DEPLOYER_PRIVATE_KEY: &str = "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";
/// Universal Deployer Contract address on Starknet
pub const UDC_ADDRESS: &str = "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";
/// Default port for Madara node
pub const MADARA_PORT: &str = "19944";
/// Path to the Madara binary for testing
pub const MADARA_BINARY_PATH: &str = "../../../../target/debug/madara";
/// Path to the Madara configuration file for devnet setup
pub const MADARA_CONFIG_PATH: &str = "../../../../configs/presets/devnet.yaml";

// starkli class-hash crates/client/settlement_client/src/starknet/test_contracts/appchain_test.casm.json
pub const APPCHAIN_CONTRACT_CASM_HASH: &str = "0x07f36e830605ddeb7c4c094639b628de297cbf61f45385b1fc3231029922b30b";
// starkli class-hash crates/client/settlement_client/src/starknet/test_contracts/messaging_test.casm.json
pub const MESSAGING_CONTRACT_CASM_HASH: &str = "0x077de37b708f9abe01c1a797856398c5e1e5dfde8213f884668fa37b13d77e30";

/// Type alias for a Starknet account used in tests
pub type StarknetAccount = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;
/// Type alias for transaction receipt results with error handling
pub type TransactionReceiptResult = Result<TransactionReceiptWithBlockInfo, ProviderError>;

/// Represents a running Madara process for testing
///
/// This struct manages the lifecycle of a Madara node process
/// and provides access to its port and other properties.
pub struct MadaraProcess {
    /// The child process handle for the Madara node
    pub process: Child,
    #[allow(dead_code)]
    /// Path to the Madara binary
    pub binary_path: PathBuf,
    /// Port on which the Madara node is running
    pub port: u16,
}

impl MadaraProcess {
    /// Creates a new Madara process for testing
    ///
    /// Attempts to find an available port in the range 19944-20044,
    /// starts a Madara node on that port, and waits for it to become available.
    ///
    /// # Arguments
    /// * `binary_path` - Path to the Madara binary
    ///
    /// # Returns
    /// A Result containing the MadaraProcess or an IO error
    pub fn new(binary_path: PathBuf) -> Result<Self, std::io::Error> {
        // Get the port assigned by the OS
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();

        println!("Starting Madara on port {}", port);

        let process = Command::new(&binary_path)
            .arg("--name")
            .arg("madara")
            .arg("--base-path")
            .arg("../madara-db33")
            .arg("--rpc-port")
            .arg(port.to_string())
            .arg("--rpc-cors")
            .arg("*")
            .arg("--rpc-external")
            .arg("--devnet")
            .arg("--chain-config-path")
            .arg(MADARA_CONFIG_PATH)
            .arg("--feeder-gateway-enable")
            .arg("--gateway-enable")
            .arg("--gateway-external")
            .arg("--gateway-port")
            .arg("8080")
            .arg("--no-l1-sync")
            .arg("--chain-config-override=block_time=5s,pending_block_update_time=1s")
            .spawn()?;

        wait_for_port(port, 2, 10);

        // Store the selected port in a field so it can be used elsewhere
        Ok(Self { process, binary_path, port })
    }

    /// Returns the port on which the Madara node is running
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for MadaraProcess {
    /// Cleans up the Madara process when dropped
    ///
    /// Kills the process and removes the database directory
    fn drop(&mut self) {
        if let Err(e) = self.process.kill() {
            eprintln!("Failed to kill Madara process: {}", e);
        } else {
            Command::new("rm").arg("-rf").arg("../madara-db33").status().expect("Failed to delete the madara db");
            println!("Madara process killed successfully");
        }
    }
}

/// Test context containing necessary components for Starknet testing
///
/// Provides access to the Starknet URL, account, and deployed contract addresses
pub struct TestContext {
    /// URL for the Starknet node
    pub url: Url,
    /// Account for interacting with Starknet
    pub account: StarknetAccount,
    /// Address of the deployed appchain or messaging contract
    pub deployed_appchain_contract_address: Felt,
    /// Port on which the Madara node is running
    pub port: u16,
}

/// Internal struct for holding the Madara process instance
struct MadaraInstance {
    #[allow(dead_code)]
    process: MadaraProcess,
}

lazy_static! {
    /// Mutex for the Madara instance to ensure thread safety
    static ref MADARA: Mutex<Option<MadaraInstance>> = Mutex::new(None);
    /// Mutex for the test context to ensure thread safety
    static ref TEST_CONTEXT: Mutex<Option<TestContext>> = Mutex::new(None);
    /// Once flag to ensure initialization happens only once
    static ref INIT: Once = Once::new();
    /// Lock for state update operations to prevent race conditions
    static ref STATE_UPDATE_LOCK: Mutex<()> = Mutex::new(());
}

/// Resource guard that ensures cleanup of test resources when dropped
pub struct TestGuard;

impl Drop for TestGuard {
    /// Cleans up test resources when the guard is dropped
    ///
    /// Attempts to clean up in different runtime contexts to ensure
    /// resources are properly released
    fn drop(&mut self) {
        println!("TestGuard: Cleaning up test resources...");

        // Approach 1: If we're in a Tokio runtime context, use a blocking task
        if let Ok(()) = std::panic::catch_unwind(|| {
            if tokio::runtime::Handle::try_current().is_ok() {
                // We're in a tokio runtime
                tokio::task::spawn_blocking(|| {
                    // Create a new runtime in this separate thread
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to build runtime for cleanup");

                    rt.block_on(async {
                        // Clean up resources
                        let mut madara_guard = MADARA.lock().await;
                        *madara_guard = None;

                        let mut context = TEST_CONTEXT.lock().await;
                        *context = None;
                    });
                });
            }
        }) {
            // We successfully handled cleanup in a tokio context
            println!("TestGuard: Cleanup initiated in tokio context");
            return;
        }

        // Approach 2: Fall back to creating a new runtime
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => {
                rt.block_on(async {
                    // Clean up resources
                    let mut madara_guard = MADARA.lock().await;
                    *madara_guard = None;

                    let mut context = TEST_CONTEXT.lock().await;
                    *context = None;
                });
                println!("TestGuard: Cleanup completed with new runtime");
            }
            Err(e) => {
                eprintln!("TestGuard: Failed to create runtime for cleanup: {}", e);
            }
        }
    }
}

/// Initializes a test context for appchain testing
///
/// Starts a Madara node if not already running, creates a Starknet account,
/// and deploys the appchain test contract.
///
/// # Returns
/// A TestGuard that will clean up resources when dropped and a Result
///
/// # Errors
/// Returns an error if initialization fails
pub async fn init_test_context() -> anyhow::Result<TestGuard> {
    // First, ensure any existing Madara instance is dropped
    {
        let mut madara_guard = MADARA.lock().await;
        *madara_guard = None;
    }

    // Then initialize new instance
    let mut madara_guard = MADARA.lock().await;

    *madara_guard = Some(MadaraInstance { process: MadaraProcess::new(PathBuf::from(MADARA_BINARY_PATH))? });

    // Get the port from the Madara instance
    let port = madara_guard.as_ref().unwrap().process.port();

    // Initialize test context
    let mut context = TEST_CONTEXT.lock().await;
    if context.is_none() {
        let account = starknet_account_with_port(port)?;
        let deployed_appchain_contract_address =
            deploy_contract(&account, APPCHAIN_CONTRACT_SIERRA, APPCHAIN_CONTRACT_CASM_HASH).await?;

        *context = Some(TestContext {
            url: Url::parse(format!("http://127.0.0.1:{}", port).as_str())?,
            account,
            deployed_appchain_contract_address,
            port,
        });
    }

    Ok(TestGuard)
}

/// Retrieves the current test context
///
/// # Returns
/// A Result containing the TestContext or an error if not initialized
///
/// # Errors
/// Returns an error if the test context has not been initialized
pub async fn get_test_context() -> anyhow::Result<TestContext> {
    let context = TEST_CONTEXT.lock().await;
    match context.as_ref() {
        Some(ctx) => {
            let account = starknet_account_with_port(ctx.port)?;
            Ok(TestContext {
                url: ctx.url.clone(),
                account,
                deployed_appchain_contract_address: ctx.deployed_appchain_contract_address,
                port: ctx.port,
            })
        }
        None => Err(anyhow::anyhow!("Test context not initialized")),
    }
}

/// Initializes a test context for messaging contract testing
///
/// Similar to init_test_context, but deploys the messaging test contract
/// instead of the appchain contract.
///
/// # Returns
/// A TestGuard that will clean up resources when dropped and a Result
///
/// # Errors
/// Returns an error if initialization fails
pub async fn init_messaging_test_context() -> anyhow::Result<TestGuard> {
    {
        let mut madara_guard = MADARA.lock().await;
        *madara_guard = None;
    }

    // First, ensure Madara is running
    let mut madara_guard = MADARA.lock().await;
    if madara_guard.is_none() {
        *madara_guard = Some(MadaraInstance { process: MadaraProcess::new(PathBuf::from(MADARA_BINARY_PATH))? });
    }

    // Get the port from the Madara instance
    let port = madara_guard.as_ref().unwrap().process.port();

    // Then initialize the test context if needed
    let mut context = TEST_CONTEXT.lock().await;
    if context.is_none() {
        let account = starknet_account_with_port(port)?;
        let deployed_appchain_contract_address =
            deploy_contract(&account, MESSAGING_CONTRACT_SIERRA, MESSAGING_CONTRACT_CASM_HASH).await?;

        *context = Some(TestContext {
            url: Url::parse(format!("http://127.0.0.1:{}", port).as_str())?,
            account,
            deployed_appchain_contract_address,
            port,
        });
    }

    Ok(TestGuard)
}

/// Creates a Starknet account connected to a specific port
///
/// # Arguments
/// * `port` - The port to connect to
///
/// # Returns
/// A Result containing the Starknet account or an error
///
/// # Errors
/// Returns an error if account creation fails
pub fn starknet_account_with_port(port: u16) -> anyhow::Result<StarknetAccount> {
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(format!("http://127.0.0.1:{}", port).as_str())?));
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(Felt::from_str(DEPLOYER_PRIVATE_KEY)?));
    let mut account = SingleOwnerAccount::new(
        provider,
        signer,
        Felt::from_str(DEPLOYER_ADDRESS)?,
        // MADARA_DEVNET
        Felt::from_str("0x4D41444152415F4445564E4554")?,
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Pending));
    Ok(account)
}

/// Creates a Starknet account with the default port
///
/// This is a fallback method that should be avoided in favor of
/// starknet_account_with_port with the correct port.
///
/// # Returns
/// A Result containing the Starknet account or an error
///
/// # Errors
/// Returns an error if account creation fails
pub fn starknet_account() -> anyhow::Result<StarknetAccount> {
    // This is a fallback that should ideally not be used directly
    // Better to use starknet_account_with_port with the correct port
    eprintln!(
        "Warning: Using starknet_account() with default port. Consider using starknet_account_with_port() instead."
    );
    starknet_account_with_port(MADARA_PORT.parse().unwrap())
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
            calldata: vec![Felt::from(update.block_number), update.global_root, update.block_hash],
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
pub async fn fire_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("fire_event")?,
            calldata: vec![],
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
pub async fn cancel_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("set_is_canceled")?,
            calldata: vec![Felt::ONE],
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
pub async fn deploy_contract(account: &StarknetAccount, sierra: &[u8], casm_hash: &str) -> anyhow::Result<Felt> {
    let contract_artifact: SierraClass = serde_json::from_slice(sierra)?;
    let flattened_class = contract_artifact.flatten()?;
    let result = account.declare_v2(Arc::new(flattened_class), Felt::from_str(casm_hash)?).send().await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let deployment = account
        .execute_v3(vec![Call {
            to: Felt::from_str(UDC_ADDRESS)?,
            selector: get_selector_from_name("deployContract")?,
            calldata: vec![result.class_hash, Felt::ZERO, Felt::ZERO, Felt::ZERO],
        }])
        .send()
        .await?;
    let deployed_contract_address =
        get_deployed_contract_address(deployment.transaction_hash, account.provider()).await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(deployed_contract_address)
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

/// Waits for a port to become available
///
/// # Arguments
/// * `port` - The port to check
/// * `timeout_secs` - Timeout in seconds for each connection attempt
/// * `max_retries` - Maximum number of connection attempts
///
/// # Returns
/// A boolean indicating whether the port became available
fn wait_for_port(port: u16, timeout_secs: u64, max_retries: u32) -> bool {
    let mut attempts = 0;
    println!("Waiting for port {} to be available...", port);

    while attempts < max_retries {
        if check_port(port, timeout_secs) {
            println!("Port {} is now available! (attempt {}/{})", port, attempts + 1, max_retries);
            return true;
        }

        attempts += 1;
        if attempts < max_retries {
            println!("Port {} not available, retrying... (attempt {}/{})", port, attempts, max_retries);
            thread::sleep(Duration::from_secs(timeout_secs));
        }
    }

    println!("Port {} not available after {} attempts", port, max_retries);
    false
}

/// Checks if a port is available by attempting to connect to it
///
/// # Arguments
/// * `port` - The port to check
/// * `timeout_secs` - Timeout in seconds for the connection attempt
///
/// # Returns
/// A boolean indicating whether the port is available
fn check_port(port: u16, timeout_secs: u64) -> bool {
    TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(timeout_secs))
        .is_ok()
}

/// Returns the state update lock for thread synchronization
///
/// This lock is used to prevent race conditions when updating state.
///
/// # Returns
/// A reference to the state update mutex
pub fn get_state_update_lock() -> &'static Mutex<()> {
    &STATE_UPDATE_LOCK
}

/// Cleans up the test context
///
/// Releases the Madara instance and test context resources.
pub async fn cleanup_test_context() {
    let mut madara_guard = MADARA.lock().await;
    *madara_guard = None;

    let mut context = TEST_CONTEXT.lock().await;
    *context = None;
}
