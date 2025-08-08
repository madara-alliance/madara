use std::str::FromStr;

use crate::services::helpers::get_file_path;
use crate::services::helpers::NodeRpcMethods;
use crate::services::helpers::TransactionFinalityStatus;
use crate::setup::ChainSetup;
use crate::setup::SetupConfigBuilder;
use bootstrapper::contract_clients::eth_bridge::StarknetLegacyEthBridge;
use bootstrapper::contract_clients::utils::read_erc20_balance;
use bootstrapper::tests::*;
use rstest::*;
use crate::services::constants::*;
use anyhow::Error;

// Async fixture that takes arguments from the test
#[fixture]
async fn setup_chain(#[default("")] test_name: &str) -> ChainSetup {
    // Load environment variables from .env.e2e file
    // This loads .env.e2e from the current directory
    dotenvy::from_filename_override(".env.e2e").expect("Failed to load the .env file");

    // Setting Config!
    println!("Running {}", test_name);
    let setup_config = SetupConfigBuilder::new(None).test_config_l2(test_name).unwrap();
    println!("Running setup");

    // Running Chain
    let mut setup_struct = ChainSetup::new(setup_config).unwrap();
    match setup_struct.setup(test_name).await {
        Ok(()) => println!("✅ Setup completed successfully"),
        Err(e) => {
            println!("❌ Setup failed: {}", e);
            panic!("Setup failed: {}", e);
        }
    }

    setup_struct
}

#[rstest]
#[case("ethbridge")]
#[tokio::test]
async fn eth_bridge_test(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    use bootstrapper::tests::deposit_and_call_withdraw_for_eth;

    // Ensuring setup stays in scope
    let setup = setup_chain.await;
    println!("Begining Test...");
    // Testing begins here!
    // Test here!
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Deposit Both Bridges test here!
    let config_file_path = get_file_path(BOOTSTRAPPER_CONFIG);
    let (l2_account_balance_before_withdraw, transaction_hash)
        = deposit_and_call_withdraw_for_eth(config_file_path.clone()).await.unwrap();

    println!("HEREHRHERHHREHHRHEHHR txn hash {}", transaction_hash.to_hex_string());
    // Fetch the block number for the transaction
    // Wait for that block number to be settled on L1
    // Check balances!

    use tokio::time::Duration;
    use tokio::time::sleep;

    // run this with-in a timeout!
    if let Some(madara_service) = setup.lifecycle_manager.madara_service() {
        sleep(Duration::from_secs(400)).await;
        let txn_finality = madara_service.get_transaction_finality(transaction_hash.to_hex_string().as_str()).await.unwrap();

        if txn_finality == TransactionFinalityStatus::AcceptedOnL1 {
            // Let's check the balances and compare
            println!("Transaction finalized");

            let l2_account_balance_after_withdraw = get_l2_account_eth_balance(config_file_path).await.unwrap();

            // L2 balance decreases
            println!("L2 balance before withdraw: {}", l2_account_balance_before_withdraw);
            println!("L2 balance after withdraw: {}", l2_account_balance_after_withdraw);
            assert!(l2_account_balance_after_withdraw < l2_account_balance_before_withdraw);

        } else {
            println!("Transaction not finalized");
            panic!("This should not have happened")
        }
    }

    println!("Test Completed");

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all(&test_name) {
        eprintln!("Failed to delete directory: {}", err);
    }
}

use alloy::{
    network::TransactionBuilder,
    primitives::{address, Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
};
use color_eyre::Result;

use starknet::{
    accounts::{Account, ExecutionEncoding, SingleOwnerAccount},
    core::{
        chain_id,
        types::{Call, Felt},
        utils::get_selector_from_name,
    },
    providers::{
        jsonrpc::{HttpTransport, JsonRpcClient},
        Url,
    },
    signers::{LocalWallet, SigningKey},
};


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


#[rstest]
#[case("ethbridge2")]
#[tokio::test]
async fn eth_bridge_test_2(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    // Ensuring setup stays in scope
    let setup = setup_chain.await;
    println!("Begining Test...");
    // Testing begins here!
    // Test here!
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Deposit Both Bridges test here!
    let config_file_path = get_file_path(BOOTSTRAPPER_CONFIG);

    const L2_RPC_URL: &str = "http://localhost:9944"; // Madara RPC URL
    const L2_CHAIN_ID_STRING: &str = "MADARA"; // Madara Chain ID
    const L2_ACCOUNT_ADDRESS: &str = "0xabc"; // Hex Madara Account Address
    const L2_ACCOUNT_PRIVATE_KEY: &str = "0xabc"; // Hex Madara Account Private Key

    const L2_ETH_TOKEN_ADDRESS: &str = "0xabc"; // Hex Madara ETH TOKEN Address
    const L2_ETH_BRIDGE_ADDRESS: &str = "0xabc"; // Hex Madara ETH BRIDGE Address

    const L1_RPC_URL: &str = "http://localhost:9944"; // L1 RPC URL
    const L1_CHAIN_ID_STRING: &str = "L1"; // L1 Chain ID
    const L1_ACCOUNT_ADDRESS: &str = "0xabc"; // Hex L1 Account Address
    const L1_ACCOUNT_PRIVATE_KEY: &str = "0xabc"; // Hex L1 Account Private Key

    const L1_ETH_TOKEN_ADDRESS: &str = "0xabc"; // Hex L1 ETH TOKEN Address
    const L1_ETH_BRIDGE_ADDRESS: &str = "0xabc"; // Hex L1 ETH BRIDGE Address

    // STEP 1 : Create the L2 Provider, Account, ETH Token & Bridge
        let l2_provider = JsonRpcClient::new(HttpTransport::new(
                Url::parse(L2_RPC_URL).unwrap(),
        ));

        let l2_signer = LocalWallet::from(SigningKey::from_secret_scalar(
                Felt::from_hex(L2_ACCOUNT_PRIVATE_KEY).unwrap(),
        ));

        let l2_address = Felt::from_hex(L2_ACCOUNT_ADDRESS).unwrap();
        let l2_account = SingleOwnerAccount::new(
            &l2_provider,
            l2_signer,
            l2_address,
            Felt::from_str(L2_CHAIN_ID_STRING).unwrap(),
            ExecutionEncoding::New,
        );

        let l2_eth_token_address = Felt::from_str(L2_ETH_TOKEN_ADDRESS).unwrap();

        let l2_eth_bridge_address = Felt::from_str(L2_ETH_BRIDGE_ADDRESS).unwrap();


    // STEP 2 : Getting the Account's ETH Token Balance before Deposit

        let l2_account_eth_balance_before_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await;


    // STEP 3 : Get the L1 provider, account, eth bridge address

        let l1_signer = PrivateKeySigner::from_str(L1_ACCOUNT_PRIVATE_KEY).unwrap();
        let l1_provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(l1_signer.clone())
            .connect_http(L1_RPC_URL.parse()?);

        let bridge_contract = StarknetEthBridge::new(L1_ETH_BRIDGE_ADDRESS, &l1_provider);

        let deposit_amount = U256::from(100_000_000_000_000_000u64); // 0.1 ETH
        let fee_amount = U256::from(10_000_000_000_000_000u64); // 0.01 ETH fee
        let total_value = deposit_amount + fee_amount; // Total ETH to send

        let tx_hash = bridge_contract
            .deposit(deposit_amount, l2_recipient)
            .value(total_value) // msg.value = deposit + fee
            .send()
            .await?
            .confirmations(3)
            .await
            .unwrap();

    // STEP 4 : Getting the Account's ETH Token Balance after Deposit

        // TODO: sleep for atleast 3 blocks ?
        let l2_account_eth_balance_after_deposit = l2_read_token_balance(
            &l2_provider,
            l2_eth_token_address,
            l2_address,
        ).await;


    // STEP 5 : balance after > balance before
    assert!(l2_account_eth_balance_after_deposit > l2_account_eth_balance_before_deposit,
        "Balance after deposit should be greater than balance before deposit");
    println!("ETH Deposit successful");




    // // STEP 3 : Calling Deposit on L2 Account for ETH Token

    //     let withdraw_result = l2_account.execute_v3(vec![Call {
    //         to: l2_eth_bridge_address,
    //         selector: get_selector_from_name("withdraw").unwrap(),
    //         calldata: vec![
    //             l2_eth_token_address,
    //             Felt::from_dec_str("1000000000000000000000").unwrap(),
    //             Felt::ZERO,
    //         ],
    //     }])
    //     .send()
    //     .await
    //     .unwrap();






    // let tst_token_address =
    //     Felt::from_hex("07394cbe418daa16e42b87ba67372d4ab4a5df0b05c6e554d158458ce245bc10").unwrap();




















    // let (l2_account_balance_before_withdraw, transaction_hash)
    //     = deposit_and_call_withdraw_for_eth(config_file_path.clone()).await.unwrap();

























    // println!("HEREHRHERHHREHHRHEHHR txn hash {}", transaction_hash.to_hex_string());
    // // Fetch the block number for the transaction
    // // Wait for that block number to be settled on L1
    // // Check balances!

    // use tokio::time::Duration;
    // use tokio::time::sleep;

    // // run this with-in a timeout!
    // if let Some(madara_service) = setup.lifecycle_manager.madara_service() {
    //     sleep(Duration::from_secs(400)).await;
    //     let txn_finality = madara_service.get_transaction_finality(transaction_hash.to_hex_string().as_str()).await.unwrap();

    //     if txn_finality == TransactionFinalityStatus::AcceptedOnL1 {
    //         // Let's check the balances and compare
    //         println!("Transaction finalized");

    //         let l2_account_balance_after_withdraw = get_l2_account_eth_balance(config_file_path).await.unwrap();

    //         // L2 balance decreases
    //         println!("L2 balance before withdraw: {}", l2_account_balance_before_withdraw);
    //         println!("L2 balance after withdraw: {}", l2_account_balance_after_withdraw);
    //         assert!(l2_account_balance_after_withdraw < l2_account_balance_before_withdraw);

    //     } else {
    //         println!("Transaction not finalized");
    //         panic!("This should not have happened")
    //     }
    // }

    // println!("Test Completed");

    // // Delete the created directory
    // if let Err(err) = std::fs::remove_dir_all(&test_name) {
    //     eprintln!("Failed to delete directory: {}", err);
    // }
}


use starknet::providers::Provider;
use starknet_core::types::FunctionCall;
use starknet_core::types::BlockId;
use starknet_core::types::BlockTag;

// pub async fn l2_read_token_balance(
//     rpc: &JsonRpcClient<HttpTransport>,
//     contract_address: Felt,
//     account_address: Felt,
// ) -> Vec<Felt> {
//     rpc.call(
//         FunctionCall {
//             contract_address,
//             entry_point_selector: get_selector_from_name("balanceOf").unwrap(),
//             calldata: vec![account_address],
//         },
//         BlockId::Tag(BlockTag::Latest),
//     )
//     .await
//     .unwrap()
// }











#[rstest]
#[case("erc20bridge")]
#[tokio::test]
async fn erc20_bridge_test(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    use bootstrapper::tests::deposit_and_call_withdraw_for_erc20;

    // Ensuring setup stays in scope
    let setup = setup_chain.await;
    println!("Begining Test...");
    // Testing begins here!
    // Test here!
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Deposit Both Bridges test here!
    let config_file_path = get_file_path(BOOTSTRAPPER_CONFIG);
    let (l2_account_balance_before_withdraw, transaction_hash)
        = deposit_and_call_withdraw_for_erc20(config_file_path.clone()).await.unwrap();

    println!("HEREHRHERHHREHHRHEHHR txn hash {}", transaction_hash.to_hex_string());
    // Fetch the block number for the transaction
    // Wait for that block number to be settled on L1
    // Check balances!

    use tokio::time::Duration;
    use tokio::time::sleep;

    // run this with-in a timeout!
    if let Some(madara_service) = setup.lifecycle_manager.madara_service() {
        sleep(Duration::from_secs(560)).await;
        let txn_finality = madara_service.get_transaction_finality(transaction_hash.to_hex_string().as_str()).await.unwrap();

        if txn_finality == TransactionFinalityStatus::AcceptedOnL1 {
            // Let's check the balances and compare
            println!("Transaction finalized");

            let l2_account_balance_after_withdraw = get_l2_account_erc20_balance(config_file_path).await.unwrap();

            // L2 balance decreases
            println!("L2 balance before withdraw: {}", l2_account_balance_before_withdraw);
            println!("L2 balance after withdraw: {}", l2_account_balance_after_withdraw);
            assert!(l2_account_balance_after_withdraw < l2_account_balance_before_withdraw);

        } else {
            println!("Transaction not finalized");
            panic!("This should not have happened")
        }
    }

    println!("Test Completed");

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all(&test_name) {
        eprintln!("Failed to delete directory: {}", err);
    }
}
