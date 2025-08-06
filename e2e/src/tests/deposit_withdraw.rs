use crate::services::helpers::get_file_path;
use crate::setup::ChainSetup;
use crate::setup::SetupConfigBuilder;
use bootstrapper::contract_clients::eth_bridge::StarknetLegacyEthBridge;
use bootstrapper::contract_clients::utils::read_erc20_balance;
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
#[case("e2esetup")]
#[tokio::test]
async fn e2e_test_setup(
    #[case] test_name: &str,
    #[future]
    #[with(test_name)]
    setup_chain: ChainSetup,
) {
    use bootstrapper::tests::deposit_both_bridges;

    // Ensuring setup stays in scope
    let _setup = setup_chain.await;
    println!("Begining Test...");
    // Testing begins here!
    // Test here!
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Deposit Both Bridges test here!
    let x = get_file_path(BOOTSTRAPPER_CONFIG);
    let y = deposit_both_bridges(x).await;

    match y {
        Ok(_) => println!("✅ Deposit Both Bridges completed successfully"),
        Err(e) => {
            println!("❌ Deposit Both Bridges failed: {}", e);
            panic!("Deposit Both Bridges failed: {}", e);
        }
    }

    println!("Test Completed");

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all(&test_name) {
        eprintln!("Failed to delete directory: {}", err);
    }
}
