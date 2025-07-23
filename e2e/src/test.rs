use rstest::rstest;

#[rstest]
#[tokio::test]
async fn e2e_test_setup() {
    use crate::setup::{Setup,SetupConfigBuilder};
    use tokio::time::Duration;
    use tokio::time::sleep;
    use crate::services::anvil::AnvilConfigBuilder;
    use crate::setup::DEFAULT_DATA_DIR;

    let anvil_config = AnvilConfigBuilder::new()
        .port(8545)
        .block_time(1_f64)
        .load_state(format!("{}/anvil.json", DEFAULT_DATA_DIR))
        .build();

    let setup_config = SetupConfigBuilder::new(None).build_l2_config().unwrap()
        .builder().anvil_config(anvil_config).build();

    println!("Running setup");

    // This will panic with the actual error message if it fails
    let mut setup_struct = Setup::new(setup_config).unwrap();

    match setup_struct.setup().await {
        Ok(()) => println!("✅ Setup completed successfully"),
        Err(e) => {
            println!("❌ Setup failed: {}", e);
            // Manual cleanup before panic
            // if let Err(cleanup_err) = setup_struct.stop_all().await {
            //     eprintln!("Failed to cleanup: {}", cleanup_err);
            // }
            panic!("Setup failed: {}", e); // This will show your actual error
        }
    }

    // Test here!
    sleep(Duration::from_secs(500)).await;

    // Delete the created directory
    if let Err(err) = std::fs::remove_dir_all("data_e2e_test_setup") {
        eprintln!("Failed to delete directory: {}", err);
    }

}
