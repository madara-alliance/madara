use rstest::*;
use starknet::accounts::Call;
use starknet_signers::{LocalWallet, SigningKey};
use tokio::time::{sleep, Duration, Instant};

use std::str::FromStr;

use alloy::network::EthereumWallet;
use starknet_accounts::ExecutionV1;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use crate::services::constants::*;
use crate::services::helpers::get_database_path;
use crate::services::helpers::get_file_path;
use starknet::accounts::Account;
use crate::setup::SetupConfig;
use starknet_core::types::FunctionCall;
use crate::services::helpers::TransactionFinalityStatus;
use starknet_core::types::BlockTag;
use starknet::providers::Provider;
use starknet::core::types::InvokeTransactionResult;

use starknet_core::types::BlockId;
use starknet_core::utils::get_selector_from_name;

use alloy::{
    primitives::U256,
    sol,
};

use starknet::{
    accounts::{ExecutionEncoding, SingleOwnerAccount},
    core::types::Felt,
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Url,
    },
};
use crate::services::helpers::NodeRpcMethods;




#[allow(unused_imports)]
use super::setup::setup_chain;
#[allow(unused_imports)]
use crate::setup::ChainSetup;

#[rstest]
#[case("ethbridgedeposit")]
#[tokio::test]
async fn test_eth_bridge_deposit(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    // Ensuring setup stays in scope
    let setup = setup_chain.await;

    let test_config = setup.config().clone();

    // PART 1

    // Deposit ETH bridge test here !
    // L1 -> L2

    // STEP 1 : Getting the Account's ETH Token Balance before Deposit
        let l2_provider = JsonRpcClient::new(HttpTransport::new(
            test_config.get_madara_config().rpc_endpoint()
        ));

        let l2_chain_id = l2_provider.chain_id().await.unwrap();

        let l2_address = Felt::from_hex(L2_ACCOUNT_ADDRESS).unwrap();
        let l2_private_key = Felt::from_hex(L2_ACCOUNT_PRIVATE_KEY).unwrap();

        let l2_eth_token_address = Felt::from_str(L2_ETH_TOKEN_ADDRESS).unwrap();
        let l2_eth_bridge_address = Felt::from_str(L2_ETH_BRIDGE_ADDRESS).unwrap();

        let l2_account_eth_balance_before_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await[0];

        let l2_signing_key = SigningKey::from_secret_scalar(l2_private_key);
        let l2_signer = LocalWallet::from(l2_signing_key);

        let mut l2_account = SingleOwnerAccount::new(
                &l2_provider,
                l2_signer,
                l2_address,
                l2_chain_id,
                ExecutionEncoding::New,
            );
        l2_account.set_block_id(BlockId::Tag(BlockTag::Latest));

    // STEP 2 : Get the L1 provider, account, eth bridge address and do deposit
        let l1_signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY).unwrap();
        let l1_wallet = EthereumWallet::from(l1_signer.clone());

        let l1_provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(l1_wallet)
            .on_http(test_config.get_anvil_config().endpoint());

        let l1_bridge_contract = StarknetEthBridge::new(L1_ETH_BRIDGE_ADDRESS.parse().unwrap(), &l1_provider);

        let l2_recipient = U256::from_str(l2_address.to_hex_string().as_str()).unwrap();
        let deposit_amount = U256::from(100_000_000_000_000_000u64); // 0.1 ETH
        let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee
        let total_value = deposit_amount + fee_amount; // Total ETH to send

        let txn = l1_bridge_contract
            .deposit(deposit_amount, l2_recipient)
            .value(total_value) // msg.value = deposit + fee
            .send()
            .await.unwrap();

        let txn_receipt = txn.with_required_confirmations(3).get_receipt().await.unwrap();
        let deposit_txn_hash = txn_receipt.transaction_hash.to_string();
        println!("Deposit Transaction Hash : {:?}", deposit_txn_hash);

    // STEP 3 : Wait for a pre-defined time, which ensures that by that time Madara must have processed the L1 Message
        sleep(Duration::from_secs(10)).await;

    // STEP 4 : Getting the Account's ETH Token Balance after Deposit
        let l2_account_eth_balance_after_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await[0];

    // STEP 5 : balance after > balance before
        println!("Balance before eth deposit: {}", l2_account_eth_balance_before_deposit);
        println!("Balance after eth deposit: {}", l2_account_eth_balance_after_deposit);
        assert!(l2_account_eth_balance_after_deposit > l2_account_eth_balance_before_deposit,
            "ETH: Balance after deposit should be greater than balance before deposit");
        println!("ETH Deposit successful");

    // PART 2

    // Withdraw ETH bridge test here !
    // L2 -> L1

    // STEP 1 : Define new balance

        let l2_account_eth_balance_before_withraw = l2_account_eth_balance_after_deposit;

    // STEP 2 : Do Withdraw!

        // Prepare L1 recipient address (ensure it's a valid Felt)
        let l1_recipient_felt = Felt::from_hex(L1_ACCOUNT_ADDRESS).unwrap();
        let withdraw_amount = Felt::from(5);

        // Call initiate_withdraw on the bridge
        println!("Initiating withdrawal...");
        // Prepare the call
        let call = Call {
            to: l2_eth_bridge_address,
            selector: starknet::core::utils::get_selector_from_name("initiate_withdraw").unwrap(),
            calldata: vec![
                l1_recipient_felt, // L1 recipient address
                withdraw_amount,       // Amount to withdraw (low part)
                Felt::ZERO,   // Amount to withdraw (high part, for u256)
            ],
        };

        // Execute the transaction
        let result = l2_account
            .execute_v1(vec![call])
            .send()
            .await.unwrap();

        let withdraw_transaction_hash = result.transaction_hash;

        // run this with-in a timeout!
        if let Some(madara_service) = setup.lifecycle_manager.madara_service() {
            let start_time = Instant::now();
            let timeout_duration = Duration::from_secs(400);
            let polling_interval = Duration::from_secs(12);

            loop {
                // Check if we've exceeded the timeout
                if start_time.elapsed() >= timeout_duration {
                    println!("Timeout reached after 400 seconds - transaction not finalized");
                    panic!("Transaction finality check timed out after 400 seconds");
                }

                println!("Checking transaction finality... (elapsed: {:?})", start_time.elapsed());

                match madara_service.get_transaction_finality(withdraw_transaction_hash.to_hex_string().as_str()).await {
                    Ok(txn_finality) => {
                        println!("Transaction status: {:?}", txn_finality);

                        if txn_finality == TransactionFinalityStatus::AcceptedOnL1 {
                            // Transaction is finalized! Let's check the balances and compare
                            println!("Transaction finalized successfully after {:?}", start_time.elapsed());

                            let l2_account_eth_balance_after_withdraw = l2_read_token_balance(
                                &l2_provider,
                                l2_eth_token_address,
                                l2_address,
                            ).await[0];

                            // L2 balance should decrease after withdrawal
                            println!("L2 balance before withdraw: {}", l2_account_eth_balance_before_withraw);
                            println!("L2 balance after withdraw: {}", l2_account_eth_balance_after_withdraw);

                            // Fix: balance should decrease, so before > after
                            assert!(l2_account_eth_balance_before_withraw > l2_account_eth_balance_after_withdraw);

                            // Exit the loop successfully
                            break;
                        }

                        // Transaction not yet finalized, continue polling
                        println!("Transaction not yet finalized, waiting {} seconds before next check...", polling_interval.as_secs());
                    },
                    Err(e) => {
                        println!("Error checking transaction finality: {:?}", e);
                        println!("Retrying in {} seconds...", polling_interval.as_secs());
                    }
                }

                // Wait before next poll
                sleep(polling_interval).await;
            }

            println!("Transaction finality check completed successfully!");
        }

        println!("Withdrawal transaction sent!");
        println!("Transaction hash: {:#x}", result.transaction_hash);

    // STEP 6 :Delete the created directory
        let dir_path = get_file_path(test_name);
        if let Err(err) = std::fs::remove_dir_all(&dir_path) {
            eprintln!("Failed to delete directory: {}", err);
        }

}


pub async fn l2_read_token_balance(
    rpc: &JsonRpcClient<HttpTransport>,
    contract_address: Felt,
    account_address: Felt,
) -> Vec<Felt> {
    rpc.call(
        FunctionCall {
            contract_address,
            entry_point_selector: get_selector_from_name("balanceOf").unwrap(),
            calldata: vec![account_address],
        },
        BlockId::Tag(BlockTag::Latest),
    )
    .await
    .unwrap()
}
