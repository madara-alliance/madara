pub mod constants;
mod erc20_bridge;
mod eth_bridge;

use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use rstest::rstest;
use url::Url;

use crate::contract_clients::config::Clients;
use crate::tests::erc20_bridge::erc20_bridge_test_helper;
use crate::tests::eth_bridge::eth_bridge_test_helper;
use crate::{bootstrap, setup_core_contract, setup_l2, BootstrapperOutput, ConfigBuilder, ConfigFile};

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
        core_contract_address: Some(core_contract_address),
        core_contract_implementation_address: Some(core_contract_implementation_address),
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
    if !output_str.contains("1.89") {
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
