pub mod utils;

use crate::setup::SetupConfig;
use alloy::network::EthereumWallet;
use alloy::primitives::Address;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use rstest::*;
use starknet::accounts::Account;
use starknet::accounts::Call;
use starknet::accounts::ConnectedAccount;
use starknet_core::types::BlockId;
use starknet_core::types::BlockTag;
use starknet_core::utils::get_selector_from_name;
use starknet_signers::{LocalWallet, SigningKey};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use utils::*;

use alloy::{primitives::U256, sol};

use starknet::{
    accounts::{ExecutionEncoding, SingleOwnerAccount},
    core::types::Felt,
    providers::jsonrpc::{HttpTransport, JsonRpcClient},
};

use crate::setup::ChainSetup;

// Define the ERC20Token contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract ERC20Token {
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

// Define the StarkNet ETH Bridge contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract StarknetEthBridge {
        /// Main deposit function - inherited from LegacyBridge
        /// @param amount The amount of ETH to deposit (in wei)
        /// @param l2Recipient The StarkNet address to receive the funds (as uint256)
        function deposit(uint256 amount, uint256 l2Recipient) external payable;
    }
}

// Define the StarkNet ERC20 Bridge contract interface
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract StarknetTokenBridge {
        /// Main deposit function - inherited from LegacyBridge
        /// @param token The address of the token to deposit
        /// @param amount The amount of tokens to deposit
        /// @param l2Recipient The StarkNet address to receive the funds (as uint256)
        function deposit(address token, uint256 amount, uint256 l2Recipient) external payable;
    }
}

#[rstest]
#[case("deposit_withdraw")]
#[tokio::test]
async fn test_bridge_deposit_and_withdraw(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    let result = run_bridge_test(setup_chain.await).await;

    // Ensure cleanup happens regardless of test outcome
    cleanup_test_directory(test_name);

    // Propagate the result after cleanup
    match result {
        Ok(_) => println!("✅ ERC20 bridge test completed successfully"),
        Err(e) => {
            eprintln!("❌ ERC20 bridge test failed: {}", e);
            panic!("Test failed: {}", e);
        }
    }
}

async fn run_bridge_test(setup: ChainSetup) -> TestResult<()> {
    let test_config = setup.config().clone();

    // Setup L2 context
    let mut l2_context = setup_l2_context(&test_config).await?;

    // Setup L1 context
    let l1_context = setup_l1_context().await?;

    // Test ETH deposit flow (L1 -> L2)
    test_eth_deposit_flow(&l1_context, &mut l2_context, &test_config).await?;

    // Test ERC20deposit flow (L1 -> L2)
    test_erc20_deposit_flow(&l1_context, &mut l2_context, &test_config).await?;

    // Test withdrawal flow (L2 -> L1)
    test_withdrawal_flow(&l2_context, &setup).await?;

    Ok(())
}

async fn setup_l2_context(test_config: &SetupConfig) -> TestResult<L2Context> {
    let provider = JsonRpcClient::new(HttpTransport::new(test_config.get_madara_config().rpc_endpoint()));

    use starknet_providers::Provider;
    let chain_id = provider.chain_id().await.map_err(|e| format!("Failed to get L2 chain ID: {}", e))?;

    let address =
        Felt::from_hex(L2_ACCOUNT_ADDRESS).map_err(|e| format!("Failed to parse L2 account address: {}", e))?;

    let private_key =
        Felt::from_hex(L2_ACCOUNT_PRIVATE_KEY).map_err(|e| format!("Failed to parse L2 private key: {}", e))?;

    let eth_token_address =
        Felt::from_str(L2_ETH_TOKEN_ADDRESS).map_err(|e| format!("Failed to parse L2 ETH token address: {}", e))?;

    let eth_bridge_address =
        Felt::from_str(L2_ETH_BRIDGE_ADDRESS).map_err(|e| format!("Failed to parse L2 ETH bridge address: {}", e))?;

    let erc20_token_address =
        Felt::from_str(L2_ERC20_TOKEN_ADDRESS).map_err(|e| format!("Failed to parse L2 ERC20 token address: {}", e))?;

    let erc20_bridge_address = Felt::from_str(L2_ERC20_BRIDGE_ADDRESS)
        .map_err(|e| format!("Failed to parse L2 ERC20 bridge address: {}", e))?;

    let signing_key = SigningKey::from_secret_scalar(private_key);
    let signer = LocalWallet::from(signing_key);

    let mut account = SingleOwnerAccount::new(provider, signer, address, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    Ok(L2Context { account, address, eth_token_address, eth_bridge_address, erc20_token_address, erc20_bridge_address })
}

async fn setup_l1_context() -> TestResult<L1Context> {
    let eth_bridge_address = Address::from_str(L1_ETH_BRIDGE_ADDRESS)
        .map_err(|e| format!("Failed to parse L1 ETH bridge address: {}", e))?;

    let erc20_token_address = Address::from_str(L1_ERC20_TOKEN_ADDRESS)
        .map_err(|e| format!("Failed to parse L1 ERC20 token address: {}", e))?;

    let erc20_bridge_address = Address::from_str(L1_ERC20_BRIDGE_ADDRESS)
        .map_err(|e| format!("Failed to parse L1 ERC20 bridge address: {}", e))?;

    Ok(L1Context { eth_bridge_address, erc20_token_address, erc20_bridge_address })
}

async fn test_eth_deposit_flow(
    l1_context: &L1Context,
    l2_context: &mut L2Context,
    test_config: &SetupConfig,
) -> TestResult<()> {
    println!("🔄 Starting ETH deposit flow test (L1 -> L2)");

    // Get initial L2 balance
    let initial_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.eth_token_address, l2_context.address).await?;

    println!("Initial ETH L2 balance: {}", initial_l2_balance);

    // Execute L1 deposit
    let deposit_amount = U256::from(100_000_000_000_000_000u64);
    execute_eth_l1_deposit(l1_context, deposit_amount, test_config).await?;

    // Wait for message processing
    println!("⏳ Waiting for L1 message processing...");
    sleep(Duration::from_secs(10)).await;

    // Verify L2 balance increased
    let final_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.eth_token_address, l2_context.address).await?;

    println!("Final ETH L2 balance: {}", final_l2_balance);

    if final_l2_balance <= initial_l2_balance {
        return Err("L2 balance did not increase after deposit".into());
    }

    println!("✅ ETH Deposit flow completed successfully");
    Ok(())
}

async fn test_erc20_deposit_flow(
    l1_context: &L1Context,
    l2_context: &mut L2Context,
    test_config: &SetupConfig,
) -> TestResult<()> {
    println!("🔄 Starting ERC20 deposit flow test (L1 -> L2)");

    // Get initial L2 balance
    let initial_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.erc20_token_address, l2_context.address)
            .await?;

    println!("Initial ERC20 L2 balance: {}", initial_l2_balance);

    // Execute L1 deposit
    let deposit_amount = U256::from(10u64);
    execute_erc20_l1_deposit(l1_context, deposit_amount, test_config).await?;

    // Wait for message processing
    println!("⏳ Waiting for L1 message processing...");
    sleep(Duration::from_secs(10)).await;

    // Verify L2 balance increased
    let final_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.erc20_token_address, l2_context.address)
            .await?;

    println!("Final ERC20 L2 balance: {}", final_l2_balance);

    if final_l2_balance <= initial_l2_balance {
        return Err("L2 balance did not increase after deposit".into());
    }

    println!("✅ ERC20 Deposit flow completed successfully");
    Ok(())
}

async fn execute_eth_l1_deposit(
    l1_context: &L1Context,
    deposit_amount: U256,
    test_config: &SetupConfig,
) -> TestResult<()> {
    // Create provider and contracts on-demand - let compiler infer types
    let signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY)
        .map_err(|e| format!("Failed to parse L1 private key: {}", e))?;

    let wallet = EthereumWallet::from(signer.clone());

    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(test_config.get_anvil_config().endpoint());

    // Create contracts - compiler infers the complex types
    let eth_bridge_contract = StarknetEthBridge::new(l1_context.eth_bridge_address, &provider);

    let l2_recipient =
        U256::from_str(L2_ACCOUNT_ADDRESS).map_err(|e| format!("Failed to parse L2 recipient address: {}", e))?;

    let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee
    let total_amount = deposit_amount + fee_amount;

    // Execute deposit
    println!("💰 Executing ETH deposit...");
    let deposit_txn = eth_bridge_contract
        .deposit(deposit_amount, l2_recipient)
        .value(total_amount)
        .send()
        .await
        .map_err(|e| format!("Failed to send deposit transaction: {}", e))?;

    let deposit_receipt = deposit_txn
        .with_required_confirmations(3)
        .get_receipt()
        .await
        .map_err(|e| format!("Failed to get deposit transaction receipt: {}", e))?;

    println!("Deposit ETH Transaction Hash: {:?}", deposit_receipt.transaction_hash.to_string());

    Ok(())
}

async fn execute_erc20_l1_deposit(
    l1_context: &L1Context,
    deposit_amount: U256,
    test_config: &SetupConfig,
) -> TestResult<()> {
    // Create provider and contracts on-demand - let compiler infer types
    let signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY)
        .map_err(|e| format!("Failed to parse L1 private key: {}", e))?;

    let wallet = EthereumWallet::from(signer.clone());

    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(test_config.get_anvil_config().endpoint());

    // Create contracts - compiler infers the complex types
    let erc20_token_contract = ERC20Token::new(l1_context.erc20_token_address, &provider);
    let erc20_bridge_contract = StarknetTokenBridge::new(l1_context.erc20_bridge_address, &provider);

    let l2_recipient =
        U256::from_str(L2_ACCOUNT_ADDRESS).map_err(|e| format!("Failed to parse L2 recipient address: {}", e))?;

    let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee

    // Approve tokens
    println!("🔐 Approving ERC20 tokens...");
    let approve_txn = erc20_token_contract
        .approve(l1_context.erc20_bridge_address, deposit_amount)
        .send()
        .await
        .map_err(|e| format!("Failed to send approve transaction: {}", e))?;

    let approve_receipt = approve_txn
        .with_required_confirmations(3)
        .get_receipt()
        .await
        .map_err(|e| format!("Failed to get approve transaction receipt: {}", e))?;

    println!("Approve ERC20 Transaction Hash: {:?}", approve_receipt.transaction_hash.to_string());

    // Execute deposit
    println!("💰 Executing deposit...");
    let deposit_txn = erc20_bridge_contract
        .deposit(l1_context.erc20_token_address, deposit_amount, l2_recipient)
        .value(fee_amount)
        .send()
        .await
        .map_err(|e| format!("Failed to send deposit transaction: {}", e))?;

    let deposit_receipt = deposit_txn
        .with_required_confirmations(3)
        .get_receipt()
        .await
        .map_err(|e| format!("Failed to get deposit transaction receipt: {}", e))?;

    println!("Deposit ERC20 Transaction Hash: {:?}", deposit_receipt.transaction_hash.to_string());

    Ok(())
}

async fn test_withdrawal_flow(l2_context: &L2Context, setup: &ChainSetup) -> TestResult<()> {
    println!("🔄 Starting ETH & ERC20 withdrawal flow test (L2 -> L1)");

    // Get initial ETH L2 balance
    let initial_eth_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.eth_token_address, l2_context.address).await?;

    // Get initial ETH L2 balance
    let initial_erc20_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.erc20_token_address, l2_context.address)
            .await?;

    // Execute ETH withdrawal
    let eth_withdraw_hash = execute_eth_l2_withdrawal(l2_context).await?;

    sleep(Duration::from_millis(11000)).await;

    // Execute ERC20 withdrawal
    let erc20_withdraw_hash = execute_erc20_l2_withdrawal(l2_context).await?;

    // Wait for transaction finality
    wait_for_transactions_finality(setup, vec![eth_withdraw_hash, erc20_withdraw_hash]).await?;

    // Verify ETH L2 balance decreased
    let final_eth_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.eth_token_address, l2_context.address).await?;

    // Verify ERC20 L2 balance decreased
    let final_erc20_l2_balance =
        get_l2_token_balance(l2_context.account.provider(), l2_context.erc20_token_address, l2_context.address)
            .await?;

    println!("ETH L2 balance before withdraw: {}", initial_eth_l2_balance);
    println!("ETH L2 balance after withdraw: {}", final_eth_l2_balance);

    if initial_eth_l2_balance <= final_eth_l2_balance {
        return Err("ETH L2 balance did not decrease after withdrawal".into());
    }
    println!("✅ ETH Withdrawal flow completed successfully");

    println!("ERC20 L2 balance before withdraw: {}", initial_erc20_l2_balance);
    println!("ERC20 L2 balance after withdraw: {}", final_erc20_l2_balance);

    if initial_erc20_l2_balance <= final_erc20_l2_balance {
        return Err("ERC20 L2 balance did not decrease after withdrawal".into());
    }

    println!("✅ ERC20 Withdrawal flow completed successfully");

    Ok(())
}

async fn execute_eth_l2_withdrawal(l2_context: &L2Context) -> TestResult<Felt> {
    let l1_recipient_felt =
        Felt::from_hex(L1_ACCOUNT_ADDRESS).map_err(|e| format!("Failed to parse L1 recipient address: {}", e))?;

    let withdraw_amount = Felt::from(5);

    println!("🏦 Initiating ETH withdrawal...");

    let call = Call {
        to: l2_context.eth_bridge_address,
        selector: get_selector_from_name("initiate_withdraw").map_err(|e| format!("Failed to get selector: {}", e))?,
        calldata: vec![
            l1_recipient_felt, // L1 recipient address
            withdraw_amount,   // Amount to withdraw (low part)
            Felt::ZERO,        // Amount to withdraw (high part, for u256)
        ],
    };

    let result = l2_context
        .account
        .execute_v1(vec![call])
        .nonce(l2_context.account.get_nonce().await?)
        .max_fee(Felt::ZERO)
        .send()
        .await
        .map_err(|e| format!("Failed to execute withdrawal transaction: {}", e))?;

    println!("ETH Withdrawal transaction hash: {:#x}", result.transaction_hash);
    Ok(result.transaction_hash)
}

async fn execute_erc20_l2_withdrawal(l2_context: &L2Context) -> TestResult<Felt> {
    let l1_recipient_felt =
        Felt::from_hex(L1_ACCOUNT_ADDRESS).map_err(|e| format!("Failed to parse L1 recipient address: {}", e))?;

    let l1_erc20_token_address_felt =
        Felt::from_str(L1_ERC20_TOKEN_ADDRESS).map_err(|e| format!("Failed to parse L1 ERC20 token address: {}", e))?;

    let withdraw_amount = Felt::from(5);

    println!("🏦 Initiating ERC20 withdrawal...");

    let call = Call {
        to: l2_context.erc20_bridge_address,
        selector: get_selector_from_name("initiate_token_withdraw")
            .map_err(|e| format!("Failed to get selector: {}", e))?,
        calldata: vec![
            l1_erc20_token_address_felt, // L1 ERC20 token address
            l1_recipient_felt,           // L1 recipient address
            withdraw_amount,             // Amount to withdraw (low part)
            Felt::ZERO,                  // Amount to withdraw (high part, for u256)
        ],
    };

    let result = l2_context
        .account
        .execute_v1(vec![call])
        .nonce(l2_context.account.get_nonce().await?)
        .max_fee(Felt::ZERO)
        .send()
        .await
        .map_err(|e| format!("Failed to execute withdrawal transaction: {}", e))?;

    println!("ERC20 Withdrawal transaction hash: {:#x}", result.transaction_hash);
    Ok(result.transaction_hash)
}
