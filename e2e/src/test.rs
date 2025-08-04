use crate::setup::ChainSetup;
use crate::setup::SetupConfigBuilder;
use rstest::*;

// Async fixture that takes arguments from the test
#[fixture]
async fn setup_chain(#[default("")] test_name: &str) -> ChainSetup {
    // Load environment variables from .env.e2e file
    // This loads .env.e2e from the current directory
    dotenv::from_filename(".env.e2e").ok();

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

    use tokio::time::sleep;
    use tokio::time::Duration;
    sleep(Duration::from_secs(4000)).await;
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
    // Ensuring setup stays in scope
    let _setup = setup_chain.await;
    // Testing begins here!
    // Test here!
    tokio::time::sleep(tokio::time::Duration::from_secs(500)).await;

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all(&format!("data_{}", test_name)) {
        eprintln!("Failed to delete directory: {}", err);
    }
}
