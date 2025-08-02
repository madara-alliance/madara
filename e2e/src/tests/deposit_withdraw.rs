use crate::setup::ChainSetup;
use crate::setup::SetupConfigBuilder;
use rstest::*;
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


async fn deposit() -> Result<(), Error> {
    // Deposit logic here
    // Step Wise Implementation
    //
    //
    //
    Ok(())
}

async fn withdraw() -> Result<(), Error> {
    // Withdraw logic here
    Ok(())
}


#[rstest]
#[tokio::test]
#[ignore = "ignored because we have a e2e test, and this is for a local test"]
async fn deposit_and_withdraw_erc20_bridge() -> Result<(), anyhow::Error> {
    env_logger::init();
    let mut config = get_test_config_file();
    let clients = Clients::init_from_config(&config).await;
    let out = bootstrap(&mut config, &clients).await;
    let eth_token_setup = out.erc20_bridge_setup_outputs.unwrap();

    let _ = erc20_bridge_test_helper(
        &clients,
        &config,
        eth_token_setup.test_erc20_token_address,
        eth_token_setup.token_bridge,
        eth_token_setup.l2_token_bridge,
    )
    .await;

    Ok(())
}

pub async fn erc20_bridge_test_helper(
    clients: &Clients,
    arg_config: &ConfigFile,
    l2_erc20_token_address: Felt,
    token_bridge: StarknetTokenBridge,
    _l2_bridge_address: Felt,
) -> Result<(), anyhow::Error> {
    token_bridge.approve(token_bridge.bridge_address(), 100000000.into()).await;
    sleep(Duration::from_secs(arg_config.l1_wait_time.parse().unwrap())).await;
    log::info!("Approval done [✅]");
    log::info!("Waiting for message to be consumed on l2 [⏳]");
    sleep(Duration::from_secs(arg_config.cross_chain_wait_time)).await;

    let balance_before =
        read_erc20_balance(clients.provider_l2(), l2_erc20_token_address, Felt::from_str(L2_DEPLOYER_ADDRESS).unwrap())
            .await;

    token_bridge
        .deposit(
            token_bridge.address(),
            10.into(),
            U256::from_str(L2_DEPLOYER_ADDRESS).unwrap(),
            U256::from_dec_str("100000000000000").unwrap(),
        )
        .await;
    sleep(Duration::from_secs(arg_config.l1_wait_time.parse().unwrap())).await;
    log::info!("Deposit done [💰]");
    log::info!("Waiting for message to be consumed on l2 [⏳]");
    sleep(Duration::from_secs(arg_config.cross_chain_wait_time)).await;

    let balance_after =
        read_erc20_balance(clients.provider_l2(), l2_erc20_token_address, Felt::from_str(L2_DEPLOYER_ADDRESS).unwrap())
            .await;

    assert_eq!(balance_before[0] + Felt::from(10), balance_after[0]);

    // Note: we are ignoring the withdrawal tests here, it would be part of e2e where
    // we have orch running as well

    // let l1_recipient = Felt::from_hex(&arg_config.l1_deployer_address).unwrap();
    // let account =
    //     build_single_owner_account(clients.provider_l2(), &arg_config.rollup_priv_key,
    // L2_DEPLOYER_ADDRESS, false)         .await;
    //
    // log::info!("Initiated token withdraw on L2 [⏳]");
    // invoke_contract(
    //     l2_bridge_address,
    //     "initiate_token_withdraw",
    //     vec![
    //         Felt::from_bytes_be_slice(token_bridge.address().as_bytes()),
    //         l1_recipient,
    //         Felt::from_dec_str("5").unwrap(),
    //         Felt::ZERO,
    //     ],
    //     &account,
    // )
    // .await;
    //
    // sleep(Duration::from_secs(arg_config.l1_wait_time.parse().unwrap())).await;
    // log::info!("Waiting for message to be consumed on l2 [⏳]");
    // sleep(Duration::from_secs(arg_config.cross_chain_wait_time)).await;
    // sleep(Duration::from_secs(arg_config.l1_wait_time.parse().unwrap())).await;
    //
    // let l1_recipient: Address = Address::from_str(&arg_config.l1_deployer_address).unwrap();
    // let balance_before = token_bridge.token_balance(l1_recipient).await;
    // token_bridge.withdraw(token_bridge.address(), 5.into(), l1_recipient).await;
    // let balance_after = token_bridge.token_balance(l1_recipient).await;
    //
    // assert_eq!(balance_before + U256::from_dec_str("5").unwrap(), balance_after);
    //
    // log::info!("Token withdraw successful [✅]");

    anyhow::Ok(())
}
