use rstest::rstest;


#[rstest]
#[tokio::test]
async fn e2e_test_setup() {
    use crate::setup::{Setup,SetupConfigBuilder};
    use tokio::time::Duration;
    use tokio::time::sleep;

    let setup_config = SetupConfigBuilder::new().build_l2_config();

    println!("Running setup");

    // This will panic with the actual error message if it fails
    let mut setup_struct = Setup::new(setup_config).unwrap();

    match setup_struct.setup("e2e_test_setup").await {
        Ok(()) => println!("✅ Setup completed successfully"),
        Err(e) => {
            println!("❌ Setup failed: {}", e);
            // Manual cleanup before panic
            if let Err(cleanup_err) = setup_struct.stop_all().await {
                eprintln!("Failed to cleanup: {}", cleanup_err);
            }
            panic!("Setup failed: {}", e); // This will show your actual error
        }
    }

    // sleep(Duration::from_secs(5000)).await;
}
