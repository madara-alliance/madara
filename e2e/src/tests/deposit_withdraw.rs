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
