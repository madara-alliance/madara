use rstest::*;
use crate::setup::SetupConfigBuilder;

use crate::services::anvil::AnvilConfigBuilder;
use crate::services::constants::*;

use crate::setup::ChainSetup;

// Async fixture that takes arguments from the test
#[fixture]
async fn setup_chain(
    #[default("")] test_name: &str,
) -> ChainSetup {

    // Setting Config!

    println!("Running {}", test_name);

    let anvil_config = AnvilConfigBuilder::new()
        .port(8545)
        .block_time(1_f64)
        .load_state(format!("{}/anvil.json", DEFAULT_DATA_DIR))
        .build();

    let setup_config = SetupConfigBuilder::new(None)
        .build_l2_setup_config()
        .unwrap()
        .builder()
        .anvil_config(anvil_config)
        .build();

    println!("Running setup");

    // Running Chain

    let mut setup_struct = ChainSetup::new(setup_config).unwrap();

    match setup_struct.setup().await {
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
    #[future] #[with(test_name)] setup_chain: ChainSetup,
) {
    use tokio::time::Duration;
    use tokio::time::sleep;

    println!("Running   setupsetupsetupsetupsetupsetupsetupsetup");

    // Ensuring setup stays in scope
    let _ = setup_chain.await;

    println!("Running e2e_test_setupe2e_test_setupe2e_test_setup");

    // Testing begins here!

    // Test here!
    sleep(Duration::from_secs(500)).await;

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all(&format!("data_{}", test_name)) {
        eprintln!("Failed to delete directory: {}", err);
    }
}
