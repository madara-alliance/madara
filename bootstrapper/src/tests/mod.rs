pub mod constants;
pub mod erc20_bridge;
pub mod eth_bridge;

use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;

use ethers::types::Address;
use rstest::rstest;
use starkgate_manager_client::clients::StarkgateManagerContractClient;
use starkgate_registry_client::clients::StarkgateRegistryContractClient;
use starknet_erc20_client::clients::ERC20ContractClient;
use starknet_eth_bridge_client::clients::eth_bridge::StarknetEthBridgeContractClient;
use starknet_token_bridge_client::clients::StarknetTokenBridgeContractClient;
use url::Url;

use crate::contract_clients::config::Clients;
use crate::contract_clients::utils::read_erc20_balance;
use crate::tests::erc20_bridge::erc20_bridge_test_helper;
use crate::tests::eth_bridge::*;
use crate::{bootstrap, get_core_contract_client, setup_core_contract, setup_l2, BootstrapperOutput, ConfigBuilder, ConfigFile};

async fn test_setup(args: &ConfigFile, clients: &Clients) -> BootstrapperOutput {
    // Setup L1 (core contract)
    let core_contract_client = setup_core_contract(args, clients).await;

    let core_contract_address = core_contract_client.core_contract_client.address();
    let core_contract_implementation_address = core_contract_client.core_contract_client.implementation_address();

    // Create a new config with the core contract addresses
    let mut config = get_test_config_file();
    config.core_contract_address = Some(format!("{:?}", core_contract_address));
    config.core_contract_implementation_address = Some(format!("{:?}", core_contract_implementation_address));

    ensure_toolchain().expect("Not able to ensure toolchain exists.");
    wait_for_madara().await.expect("Failed to start madara!");

    // Setup L2 with the updated config
    let l2_output = setup_l2(&mut config, clients).await;

    BootstrapperOutput {
        starknet_contract_address: Some(core_contract_address),
        starknet_contract_implementation_address: Some(core_contract_implementation_address),
        ..l2_output
    }
}

#[rstest]
#[tokio::test]
#[ignore = "ignored because we have a e2e test, and this is for a local test"]
async fn deploy_bridge() -> Result<(), anyhow::Error> {
    env_logger::init();
    let mut config = get_test_config_file();
    let clients = Clients::init_from_config(&config).await;
    bootstrap(&mut config, &clients).await;

    Ok(())
}

#[rstest]
#[tokio::test]
#[ignore = "ignored because we have a e2e test, and this is for a local test"]
async fn deposit_and_withdraw_eth_bridge() -> Result<(), anyhow::Error> {
    env_logger::init();
    let mut config = get_test_config_file();
    let clients = Clients::init_from_config(&config).await;
    let out = bootstrap(&mut config, &clients).await;
    let eth_bridge_setup = out.eth_bridge_setup_outputs.unwrap();

    let _ = eth_bridge_test_helper(
        &clients,
        &config,
        eth_bridge_setup.l2_eth_proxy_address,
        eth_bridge_setup.l2_eth_bridge_proxy_address,
        eth_bridge_setup.l1_bridge,
    )
    .await;

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

#[rstest]
#[tokio::test]
async fn deposit_tests_both_bridges() -> Result<(), anyhow::Error> {
    env_logger::init();
    let config = get_test_config_file();

    // This will kill the madara when this test fails/passes
    let _port_killer = PortKiller;

    let clients = Clients::init_from_config(&config).await;
    let out = test_setup(&config, &clients).await;

    let eth_bridge_setup = out.eth_bridge_setup_outputs.unwrap();
    let eth_token_setup = out.erc20_bridge_setup_outputs.unwrap();

    let _ = eth_bridge_test_helper(
        &clients,
        &config,
        eth_bridge_setup.l2_eth_proxy_address,
        eth_bridge_setup.l2_eth_bridge_proxy_address,
        eth_bridge_setup.l1_bridge,
    )
    .await;

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

// Create a struct that will kill the process on port when dropped
struct PortKiller;
impl Drop for PortKiller {
    fn drop(&mut self) {
        kill_process_on_port(19944);
    }
}

fn kill_process_on_port(port: u16) {
    Command::new("sh")
        .arg("-c")
        .arg(format!("lsof -i :{} | grep LISTEN | awk '{{print $2}}' | xargs kill -9", port))
        .output()
        .expect("Failed to execute command");
}

fn get_test_config_file() -> ConfigFile {
    ConfigBuilder::default().build().expect("Failed to convert config builder to final config")
}

async fn wait_for_madara() -> color_eyre::Result<()> {
    let target_bin = env::var("MADARA_BOOTSTRAPPER_MADARA_BINARY_PATH").expect("failed to get binary path");
    let target_bin = PathBuf::from(target_bin);

    if !target_bin.exists() {
        panic!("No binary to run: {:?}", target_bin)
    }

    let config_path = env::var("MADARA_BOOTSTRAPPER_CONFIG_PATH").expect("failed to get config path");
    let config_path = PathBuf::from(config_path);

    if !config_path.exists() {
        panic!("Config file not found at: {:?}", config_path);
    }

    Command::new(target_bin)
        .arg("--name")
        .arg("madara")
        .arg("--base-path")
        .arg("../madara-dbs/madara_pathfinder_test_11")
        .arg("--rpc-port")
        .arg("19944")
        .arg("--rpc-cors")
        .arg("*")
        .arg("--rpc-external")
        .arg("--sequencer")
        .arg("--chain-config-path")
        .arg(config_path)
        .arg("--feeder-gateway-enable")
        .arg("--gateway-enable")
        .arg("--gateway-external")
        .arg("--l1-gas-price")
        .arg("0")
        .arg("--blob-gas-price")
        .arg("0")
        .arg("--strk-per-eth")
        .arg("1")
        .arg("--no-charge-fee")
        .arg("--rpc-admin")
        .arg("--rpc-admin-port")
        .arg("19943")
        .arg("--l1-endpoint")
        .arg("http://localhost:8545")
        .spawn()?;

    wait_for_madara_to_be_ready(Url::parse("http://localhost:19944")?).await?;

    Ok(())
}

fn ensure_toolchain() -> color_eyre::Result<()> {
    let output = Command::new("rustup").arg("toolchain").arg("list").output()?;

    let output_str = String::from_utf8_lossy(&output.stdout);
    if !output_str.contains("1.86") {
        Command::new("rustup").arg("install").arg("1.81").status()?;
    }
    Ok(())
}

pub async fn wait_for_madara_to_be_ready(rpc_url: Url) -> color_eyre::Result<()> {
    // We are fine with `expect` here as this function is called in the intial phases of the
    // program execution
    let endpoint = rpc_url.join("/health").expect("Request to health endpoint failed");
    // We would wait for about 20-25 mins for madara to be ready
    wait_for_cond(
        || async {
            let res = reqwest::get(endpoint.clone()).await?;
            res.error_for_status()?;
            Ok(true)
        },
        Duration::from_secs(30),
        10,
    )
    .await
    .expect("Could not get health of Madara");
    Ok(())
}

pub async fn wait_for_cond<F: Future<Output = color_eyre::Result<bool>>>(
    mut cond: impl FnMut() -> F,
    duration: Duration,
    attempt_number: usize,
) -> color_eyre::Result<bool> {
    let mut attempt = 0;
    loop {
        let err = match cond().await {
            Ok(result) => return Ok(result),
            Err(err) => err,
        };

        attempt += 1;
        if attempt >= attempt_number {
            panic!("No answer from the node after {attempt} attempts: {:#}", err)
        }

        tokio::time::sleep(duration).await;
    }
}


//////////////////////////////////////////////////////////////////////////////////////
/// E2E
//////////////////////////////////////////////////////////////////////////////////////


use ethereum_instance::EthereumClient;
use crate::tests::constants::L2_DEPLOYER_ADDRESS;
use crate::contract_clients::token_bridge::StarknetTokenBridge;
use crate::contract_clients::eth_bridge::StarknetLegacyEthBridge;
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::core::types::Felt;

/// From L1 -> L2
/// Deposit doesn't require state update!
/// Transaction happens on L1 core contract,
/// after which sequencer listens to it.
/// When listened, executes a transaction on L2.
/// After few mined blocks on L2, check balance of both account
/// It will be changed!
///
/// Fetch Before_Balance of L2 Account, Store in-memory
/// Call (ETH/ERC) Bridge Deposit from L1 Account
/// Wait for Madara to mint some blocks
/// Wait Emperical Delay
/// Fetch After_Balance of L1 Account, Store in-memory
/// Fetch After_Balance of L2 Account, Store in-memory
/// Compare the Before_Balances with the After_Balances
/// It should balance!
pub async fn deposit_both_bridges(file_path: PathBuf) -> color_eyre::Result<()> {
    let _ = orchestrate_deposit_to_eth_bridge(file_path.clone()).await?;
    let _ = orchestrate_deposit_to_erc20_bridge(file_path).await;

    Ok(())

}

use ethers::prelude::H160;

async fn get_eth_bridge(config: &ConfigFile, clients: &Clients) -> StarknetLegacyEthBridge {
    let core_contract_output = get_core_contract_client(config, clients);

    let eth_bridge_address_string = config.l1_eth_bridge_address.clone().unwrap();
    let eth_bridge_address = H160::from_str(eth_bridge_address_string.as_str()).expect("Failed to parse eth bridge address");

    let eth_bridge = StarknetEthBridgeContractClient::new(
        eth_bridge_address,
        core_contract_output.core_contract_client.client(),
        // IMP! We are not using Implementation, so we can put any address
        Address::default(),
    );

    StarknetLegacyEthBridge {
        eth_bridge
    }
}

async fn orchestrate_deposit_to_eth_bridge(config_file_path: PathBuf) -> color_eyre::Result<()> {
    let bootstrapper_config: ConfigFile = ConfigBuilder::from_file(config_file_path)?.merge_with_env().build().expect("Failed to convert config builder to final config");
    let clients = Clients::init_from_config(&bootstrapper_config).await;
    // let l1_provider: &EthereumClient = clients.eth_client();
    let l2_provider: &JsonRpcClient<HttpTransport> = clients.provider_l2();

    let l2_eth_address = Felt::from_str(&bootstrapper_config.l2_eth_token_proxy_address.clone().unwrap()).expect("Failed to parse l2_eth_token_proxy_address");

    let eth_bridge = get_eth_bridge(&bootstrapper_config, &clients).await;

    let balance_before: Vec<Felt> = read_erc20_balance(l2_provider, l2_eth_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;

    let result_deposit_eth_bridge = deposit_to_eth_bridge(
        bootstrapper_config.cross_chain_wait_time,
        bootstrapper_config.l1_wait_time,
        eth_bridge
    ).await;

    let balance_after: Vec<Felt> = read_erc20_balance(l2_provider, l2_eth_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;
    assert_eq!(balance_before[0] + Felt::from_dec_str("10")?, balance_after[0]);

    Ok(())
}

async fn get_erc20_bridge(config: &ConfigFile, clients: &Clients) -> StarknetTokenBridge {
    let core_contract_output = get_core_contract_client(config, clients);

    let erc20_bridge_address_string = config.l1_erc20_bridge_address.clone().unwrap();
    let erc20_bridge_address = H160::from_str(erc20_bridge_address_string.as_str()).expect("Failed to parse eth bridge address");

    let l2_erc20_address = H160::from_str(config.l1_erc20_token_address.clone().unwrap().as_str()).expect("Failed to parse eth bridge address");

    // IMP! We are not using this, so we can put any values
    let manager = StarkgateManagerContractClient::new(
       Address::default(),
       core_contract_output.core_contract_client.client(),
       // IMP! We are not using Implementation, so we can put any address
       Address::default(),
    );

    // IMP! We are not using this, so we can put any values
    let registry = StarkgateRegistryContractClient::new(
       Address::default(),
       core_contract_output.core_contract_client.client(),
       // IMP! We are not using Implementation, so we can put any address
       Address::default(),
    );

    let token_bridge = StarknetTokenBridgeContractClient::new(
       erc20_bridge_address,
       core_contract_output.core_contract_client.client(),
       // IMP! We are not using Implementation, so we can put any address
       Address::default(),
    );

    let erc20 = ERC20ContractClient::new(
       l2_erc20_address,
       core_contract_output.core_contract_client.client(),
    );

    StarknetTokenBridge {
        manager,
        registry,
        token_bridge,
        erc20
    }
}

async fn orchestrate_deposit_to_erc20_bridge(config_file_path: PathBuf) -> color_eyre::Result<()> {
    let bootstrapper_config: ConfigFile = ConfigBuilder::from_file(config_file_path)?.merge_with_env().build().expect("Failed to convert config builder to final config");
    let clients = Clients::init_from_config(&bootstrapper_config).await;
    let l2_provider: &JsonRpcClient<HttpTransport> = clients.provider_l2();

    let l2_erc20_token_address = Felt::from_str(&bootstrapper_config.l2_erc20_token_address.clone().unwrap()).expect("Failed to parse l2_test_erc20_token_address");
    let token_bridge: StarknetTokenBridge = get_erc20_bridge(&bootstrapper_config, &clients).await;

    let balance_before: Vec<Felt> = read_erc20_balance(l2_provider, l2_erc20_token_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;

    let result_deposit_erc20_bridge = deposit_to_erc20_bridge(
        bootstrapper_config.cross_chain_wait_time,
        bootstrapper_config.l1_wait_time,
        token_bridge
    ).await;

    let balance_after: Vec<Felt> = read_erc20_balance(l2_provider, l2_erc20_token_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;
    assert_eq!(balance_before[0] + Felt::from_dec_str("10")?, balance_after[0]);
    Ok(())
}


// /// From L2 -> L1
// /// Withdraw Requires state update!
// /// Transactions happen on L2
// /// And are submitted to L1 core contract
// /// via state update of that block.
// /// Will need to wait for state update of the block
// /// the transaction occured in.
// /// After a emperical block number delay,
// /// check the balance for both, it will be changed!
// async fn withdraw() -> Result<(), Error> {
//     // Withdraw logic here
//     // Step Wise Implementation
//     // Fetch Before_Balance of L1 Account, Store in-memory
//     // Fetch Before_Balance of L2 Account, Store in-memory
//     // Call (ETH/ERC) Bridge Withdraw from L2 Account
//     // Wait for Orchestrator to settle that block
//     // Wait Emperical Delay
//     // Fetch After_Balance of L1 Account, Store in-memory
//     // Fetch After_Balance of L2 Account, Store in-memory
//     // Compare the Before_Balances with the After_Balances
//     // It should balance!
//     Ok(())
// }
