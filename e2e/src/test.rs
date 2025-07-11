use crate::setup::{Setup, SetupConfig};
use rstest::rstest;

// #[rstest::fixture]
// pub(crate) async fn setup() -> SomeSetupType {
//     // based on some config :

//     let setup_config = SetupConfig::default();

//     let new_setup = Setup::new(setup_config).await;

//     new_setup.l2_setup().await;
// }
#[rstest]
#[tokio::test]
async fn e2e_test_setup() {
    let setup_config = SetupConfig::default();

    println!("Running setup");

    // This will panic with the actual error message if it fails
    let mut setup_struct = Setup::new(setup_config).unwrap();

    let x = setup_struct.setup().await;

    println!("Setup completed successfully!");

    // This is going to be the first test that will validate
    // if the system is setting up properly or not
    // System setup :

    // 1. A clean Anvil instance comes alive.
    // 2. Bootstrapper L1 setup completes.
    // 3. A clean Madara instance comes alive.
    // 4. Bootstrapper L2 setup completes.
    // 5. Madara is restarted with larger block time.
    // 6. Pathfinder setup completes.
    // 7. Orchestrator setup happens parallel to 6.
    // 8. Orchestrator runs.
}
