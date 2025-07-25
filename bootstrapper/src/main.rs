pub mod contract_clients;
pub mod helpers;
mod setup_scripts;
#[cfg(test)]
pub mod tests;
pub mod utils;

use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, ValueEnum};
use contract_clients::utils::RpcAccount;
use dotenv::dotenv;
use ethers::abi::{AbiEncode, Address};
use inline_colorization::*;
use serde::{Deserialize, Serialize};
use setup_scripts::argent::ArgentSetupOutput;
use setup_scripts::braavos::BraavosSetupOutput;
use setup_scripts::core_contract::CoreContractStarknetL1Output;
use setup_scripts::erc20_bridge::Erc20BridgeSetupOutput;
use setup_scripts::eth_bridge::EthBridgeSetupOutput;
use setup_scripts::udc::UdcSetupOutput;
use starknet::accounts::Account;
use starknet_core_contract_client::clients::StarknetCoreContractClient;
use starknet_types_core::felt::Felt;

use crate::contract_clients::config::Clients;
use crate::contract_clients::starknet_core_contract::StarknetCoreContract;
use crate::contract_clients::utils::build_single_owner_account;
use crate::setup_scripts::account_setup::account_init;
use crate::setup_scripts::argent::ArgentSetup;
use crate::setup_scripts::braavos::BraavosSetup;
use crate::setup_scripts::core_contract::CoreContractStarknetL1;
use crate::setup_scripts::erc20_bridge::Erc20Bridge;
use crate::setup_scripts::eth_bridge::EthBridge;
use crate::setup_scripts::udc::UdcSetup;
use crate::setup_scripts::upgrade_eth_token::upgrade_eth_token_to_cairo_1;
use crate::setup_scripts::upgrade_l1_bridge::upgrade_l1_bridge;
use crate::setup_scripts::upgrade_l2_bridge::upgrade_eth_bridge_to_cairo_1;
use crate::utils::banner::BANNER;
use crate::utils::{hexstring_to_address, save_to_json, JsonValueType};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BootstrapMode {
    Core,
    SetupL1,
    SetupL2,
    EthBridge,
    Erc20Bridge,
    Udc,
    Argent,
    Braavos,
    UpgradeEthBridge,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    #[clap(long)]
    config: Option<PathBuf>,
    #[clap(long, env, value_enum)]
    mode: BootstrapMode,
    #[clap(long, env)]
    output_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum CoreContractMode {
    Production,
    Dev,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigBuilder {
    pub eth_rpc: Option<String>,
    pub eth_priv_key: Option<String>,
    pub rollup_priv_key: Option<String>,
    pub rollup_seq_url: String,
    pub rollup_declare_v0_seq_url: String,
    pub eth_chain_id: u64,
    pub l1_deployer_address: String,
    pub l1_wait_time: String,
    pub sn_os_program_hash: String,
    pub config_hash_version: String,
    pub app_chain_id: String,
    pub fee_token_address: String,
    pub native_fee_token_address: String,
    pub cross_chain_wait_time: u64,
    pub l1_multisig_address: String,
    pub l2_multisig_address: String,
    pub verifier_address: String,
    pub operator_address: String,
    pub dev: bool,
    pub core_contract_mode: CoreContractMode,
    pub l2_deployer_address: Option<String>,
    pub core_contract_address: Option<String>,
    pub core_contract_implementation_address: Option<String>,
    pub udc_address: Option<String>,
    pub l1_eth_bridge_address: Option<String>,
    pub l2_eth_token_proxy_address: Option<String>,
    pub l2_eth_bridge_proxy_address: Option<String>,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            eth_rpc: Some("http://127.0.0.1:8545".to_string()),
            eth_priv_key: Some("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string()),
            rollup_priv_key: Some("0xabcd".to_string()),
            rollup_seq_url: "http://127.0.0.1:19944".to_string(),
            rollup_declare_v0_seq_url: "http://127.0.0.1:19943".to_string(),
            eth_chain_id: 31337,
            l1_deployer_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            l1_wait_time: "15".to_string(),
            sn_os_program_hash: "0x1e324682835e60c4779a683b32713504aed894fd73842f7d05b18e7bd29cd70".to_string(),
            config_hash_version: "StarknetOsConfig2".to_string(),
            app_chain_id: "MADARA_DEVNET".to_string(),
            fee_token_address: "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7".to_string(),
            native_fee_token_address: "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d".to_string(),
            cross_chain_wait_time: 20,
            l1_multisig_address: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8".to_string(),
            l2_multisig_address: "0x556455b8ac8bc00e0ad061d7df5458fa3c372304877663fa21d492a8d5e9435".to_string(),
            verifier_address: "0x000000000000000000000000000000000000abcd".to_string(),
            operator_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            dev: false,
            core_contract_mode: CoreContractMode::Dev,
            l2_deployer_address: None,
            core_contract_address: Some("0xe7f1725e7734ce288f8367e1bb143e90bb3f0512".to_string()),
            core_contract_implementation_address: Some("0x5fbdb2315678afecb367f032d93f642f64180aa3".to_string()),
            udc_address: None,
            l1_eth_bridge_address: None,
            l2_eth_token_proxy_address: None,
            l2_eth_bridge_proxy_address: None,
        }
    }
}

impl ConfigBuilder {
    pub fn from_file(path: PathBuf) -> color_eyre::Result<Self> {
        let file = File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }

    pub fn merge_with_env(mut self) -> Self {
        if let Ok(eth_rpc) = std::env::var("ETH_RPC") {
            self.eth_rpc = Some(eth_rpc);
        }
        if let Ok(eth_priv_key) = std::env::var("ETH_PRIVATE_KEY") {
            self.eth_priv_key = Some(eth_priv_key);
        }
        if let Ok(rollup_priv_key) = std::env::var("ROLLUP_PRIVATE_KEY") {
            self.rollup_priv_key = Some(rollup_priv_key);
        }
        if let Ok(rollup_seq_url) = std::env::var("ROLLUP_SEQ_URL") {
            self.rollup_seq_url = rollup_seq_url;
        }
        if let Ok(rollup_declare_v0_seq_url) = std::env::var("ROLLUP_DECLARE_V0_SEQ_URL") {
            self.rollup_declare_v0_seq_url = rollup_declare_v0_seq_url;
        }
        if let Ok(sn_os_program_hash) = std::env::var("SN_OS_PROGRAM_HASH") {
            self.sn_os_program_hash = sn_os_program_hash;
        }
        if let Ok(config_hash_version) = std::env::var("CONFIG_HASH_VERSION") {
            self.config_hash_version = config_hash_version;
        }
        self
    }

    pub fn build(self) -> color_eyre::Result<ConfigFile> {
        Ok(ConfigFile {
            eth_rpc: self
                .eth_rpc
                .ok_or_else(|| color_eyre::eyre::eyre!("ETH_RPC must be provided in config file or environment"))?,
            eth_priv_key: self.eth_priv_key.ok_or_else(|| {
                color_eyre::eyre::eyre!("ETH_PRIVATE_KEY must be provided in config file or environment")
            })?,
            rollup_priv_key: self.rollup_priv_key.ok_or_else(|| {
                color_eyre::eyre::eyre!("ROLLUP_PRIVATE_KEY must be provided in config file or environment")
            })?,
            rollup_seq_url: self.rollup_seq_url,
            rollup_declare_v0_seq_url: self.rollup_declare_v0_seq_url,
            eth_chain_id: self.eth_chain_id,
            l1_deployer_address: self.l1_deployer_address,
            l1_wait_time: self.l1_wait_time,
            sn_os_program_hash: self.sn_os_program_hash,
            config_hash_version: self.config_hash_version,
            app_chain_id: self.app_chain_id,
            fee_token_address: self.fee_token_address,
            native_fee_token_address: self.native_fee_token_address,
            cross_chain_wait_time: self.cross_chain_wait_time,
            l1_multisig_address: self.l1_multisig_address,
            l2_multisig_address: self.l2_multisig_address,
            verifier_address: self.verifier_address,
            operator_address: self.operator_address,
            dev: self.dev,
            core_contract_mode: self.core_contract_mode,
            l2_deployer_address: self.l2_deployer_address,
            core_contract_address: self.core_contract_address,
            core_contract_implementation_address: self.core_contract_implementation_address,
            udc_address: self.udc_address,
            l1_eth_bridge_address: self.l1_eth_bridge_address,
            l2_eth_token_proxy_address: self.l2_eth_token_proxy_address,
            l2_eth_bridge_proxy_address: self.l2_eth_bridge_proxy_address,
        })
    }
}
#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigFile {
    pub eth_rpc: String,
    pub eth_priv_key: String,
    pub rollup_seq_url: String,
    pub rollup_declare_v0_seq_url: String,
    pub rollup_priv_key: String,
    pub eth_chain_id: u64,
    pub l1_deployer_address: String,
    pub l1_wait_time: String,
    pub sn_os_program_hash: String,
    pub config_hash_version: String,
    pub app_chain_id: String,
    pub fee_token_address: String,
    pub native_fee_token_address: String,
    pub cross_chain_wait_time: u64,
    pub l1_multisig_address: String,
    pub l2_multisig_address: String,
    pub verifier_address: String,
    pub operator_address: String,
    pub dev: bool,
    pub core_contract_mode: CoreContractMode,
    pub l2_deployer_address: Option<String>,
    pub core_contract_address: Option<String>,
    pub core_contract_implementation_address: Option<String>,
    pub udc_address: Option<String>,
    pub l1_eth_bridge_address: Option<String>,
    pub l2_eth_token_proxy_address: Option<String>,
    pub l2_eth_bridge_proxy_address: Option<String>,
}

#[tokio::main]
pub async fn main() -> color_eyre::Result<()> {
    // a logger that can be configured by env
    env_logger::init();

    // parsing env
    dotenv().ok();

    // clap to make cli applications
    let args = CliArgs::parse();

    println!("{color_red}{}{color_reset}", BANNER);

    let mut config_file = match args.config {
        Some(path) => ConfigBuilder::from_file(path)?.merge_with_env().build()?,
        None => ConfigBuilder::default().merge_with_env().build()?,
    };

    let clients = Clients::init_from_config(&config_file).await;

    let account = match config_file.l2_deployer_address {
        Some(ref addr) => Some(
            build_single_owner_account(clients.provider_l2(), &config_file.rollup_priv_key, &addr.to_string(), false)
                .await,
        ),
        None => None,
    };

    let output = match args.mode {
        BootstrapMode::Core | BootstrapMode::SetupL1 => {
            let output = setup_core_contract(&config_file, &clients).await;

            BootstrapperOutput {
                starknet_contract_address: Some(output.core_contract_client.address()),
                starknet_contract_implementation_address: Some(output.core_contract_client.implementation_address()),
                ..Default::default()
            }
        }
        BootstrapMode::SetupL2 => setup_l2(&mut config_file, &clients).await,
        BootstrapMode::EthBridge => {
            let core_contract_client = get_core_contract_client(&config_file, &clients);
            let output = setup_eth_bridge(account, &core_contract_client, &config_file, &clients).await;
            BootstrapperOutput { eth_bridge_setup_outputs: Some(output), ..Default::default() }
        }
        BootstrapMode::Erc20Bridge => {
            let core_contract_client = get_core_contract_client(&config_file, &clients);
            let output = setup_erc20_bridge(account, &core_contract_client, &config_file, &clients).await;
            BootstrapperOutput { erc20_bridge_setup_outputs: Some(output), ..Default::default() }
        }
        BootstrapMode::Udc => {
            let output = setup_udc(account, &config_file, &clients).await;
            BootstrapperOutput { udc_setup_outputs: Some(output), ..Default::default() }
        }
        BootstrapMode::Argent => {
            let output = setup_argent(account, &config_file, &clients).await;
            BootstrapperOutput { argent_setup_outputs: Some(output), ..Default::default() }
        }
        BootstrapMode::Braavos => {
            let output = setup_braavos(
                account,
                &config_file,
                &clients,
                Felt::from_str(
                    &config_file.udc_address.clone().expect("UDC Address not available in config. Run with mode UDC"),
                )
                .expect("Unable to get UDC address"),
            )
            .await;
            BootstrapperOutput { braavos_setup_outputs: Some(output), ..Default::default() }
        }
        BootstrapMode::UpgradeEthBridge => {
            upgrade_eth_bridge(account, &config_file, &clients).await.expect("Unable to upgrade Eth bridge");
            BootstrapperOutput { ..Default::default() }
        }
    };

    let output_json =
        serde_json::to_string_pretty(&output).unwrap_or_else(|e| format!("Error serializing output: {}", e));

    // Print the output to the console
    println!("Bootstrap Output:");
    println!("{}", output_json);

    if let Some(output_file) = args.output_file {
        let file = File::create(&output_file).unwrap();
        serde_json::to_writer_pretty(file, &output).unwrap();
        println!("✅ Bootstrap output saved to {}", output_file);
    }
    Ok(())
}

fn get_core_contract_client(config_file: &ConfigFile, clients: &Clients) -> CoreContractStarknetL1Output {
    let Some(core_contract_address) = config_file.core_contract_address.clone() else {
        panic!("Core contract address is required for ETH bridge setup");
    };
    let Some(core_contract_implementation_address) = config_file.core_contract_implementation_address.clone() else {
        panic!("Core contract implementation address is required for ETH bridge setup");
    };
    let core_contract_client = StarknetCoreContractClient::new(
        hexstring_to_address(&core_contract_address),
        clients.eth_client().signer().clone(),
        hexstring_to_address(&core_contract_implementation_address),
    );
    CoreContractStarknetL1Output { core_contract_client: Box::new(StarknetCoreContract { core_contract_client }) }
}

async fn get_account<'a>(clients: &'a Clients, config_file: &'a ConfigFile) -> RpcAccount<'a> {
    log::info!("⏳ L2 State and Initialisation Started");
    let account = account_init(clients, config_file).await;
    log::info!("🔐 Account with given  private key deployed on L2. [Account Address : {:?}]", account.address());
    account
}

#[derive(Serialize, Clone, Default)]
pub struct BootstrapperOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starknet_contract_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starknet_contract_implementation_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eth_bridge_setup_outputs: Option<EthBridgeSetupOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub erc20_bridge_setup_outputs: Option<Erc20BridgeSetupOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udc_setup_outputs: Option<UdcSetupOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argent_setup_outputs: Option<ArgentSetupOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub braavos_setup_outputs: Option<BraavosSetupOutput>,
}

pub async fn bootstrap(config_file: &mut ConfigFile, clients: &Clients) -> BootstrapperOutput {
    // setup core contract (L1)
    let core_contract_client = setup_core_contract(config_file, clients).await;

    // setup L2
    let l2_output = setup_l2(config_file, clients).await;

    BootstrapperOutput {
        starknet_contract_address: Some(core_contract_client.core_contract_client.address()),
        starknet_contract_implementation_address: Some(
            core_contract_client.core_contract_client.implementation_address(),
        ),
        ..l2_output
    }
}

async fn setup_core_contract(config_file: &ConfigFile, clients: &Clients) -> CoreContractStarknetL1Output {
    let core_contract = CoreContractStarknetL1::new(config_file, clients);
    let core_contract_client = core_contract.setup().await;
    log::info!("📦 Core address : {:?}", core_contract_client.core_contract_client.address());
    log::info!(
        "📦 Core implementation address : {:?}",
        core_contract_client.core_contract_client.implementation_address()
    );
    save_to_json(
        "l1_core_contract_address",
        &JsonValueType::EthAddress(core_contract_client.core_contract_client.address()),
    )
    .unwrap();
    log::info!("✅ Core setup init for L1 successful.");
    core_contract_client
}

async fn setup_eth_bridge(
    account: Option<RpcAccount<'_>>,
    core_contract_client: &CoreContractStarknetL1Output,
    config_file: &ConfigFile,
    clients: &Clients,
) -> EthBridgeSetupOutput {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    log::info!("⏳ Starting ETH bridge deployment");
    let eth_bridge = EthBridge::new(
        account.clone(),
        account.address(),
        config_file,
        clients,
        core_contract_client.core_contract_client.as_ref(),
    );
    let eth_bridge_setup_outputs = eth_bridge.setup().await;
    log::info!("✅ ETH bridge deployment complete.");
    eth_bridge_setup_outputs
}

async fn upgrade_eth_bridge(
    account: Option<RpcAccount<'_>>,
    config_file: &ConfigFile,
    clients: &Clients,
) -> color_eyre::Result<()> {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    upgrade_eth_token_to_cairo_1(
        &account,
        clients.provider_l2(),
        Felt::from_str(
            &config_file.l2_eth_token_proxy_address.clone().expect("l2_eth_token_proxy_address not in config."),
        )?,
    )
    .await;
    upgrade_eth_bridge_to_cairo_1(
        &account,
        clients.provider_l2(),
        Felt::from_str(
            &config_file.l2_eth_bridge_proxy_address.clone().expect("l2_eth_bridge_proxy_address not in config."),
        )?,
        Felt::from_str(
            &config_file.l2_eth_token_proxy_address.clone().expect("l2_eth_token_proxy_address not in config."),
        )?,
    )
    .await;

    let l1_eth_bridge_address =
        hexstring_to_address(&config_file.l1_eth_bridge_address.clone().expect("l1_eth_bridge_address not in config."));
    upgrade_l1_bridge(l1_eth_bridge_address, config_file).await?;

    Ok(())
}

async fn setup_erc20_bridge(
    account: Option<RpcAccount<'_>>,
    core_contract_client: &CoreContractStarknetL1Output,
    config_file: &ConfigFile,
    clients: &Clients,
) -> Erc20BridgeSetupOutput {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    log::info!("⏳ Starting ERC20 token bridge deployment");
    let erc20_bridge = Erc20Bridge::new(
        account.clone(),
        account.address(),
        config_file,
        clients,
        core_contract_client.core_contract_client.as_ref(),
    );
    let erc20_bridge_setup_outputs = erc20_bridge.setup().await;
    log::info!("✅ ERC20 token bridge deployment complete.");
    erc20_bridge_setup_outputs
}

async fn setup_udc(account: Option<RpcAccount<'_>>, config_file: &ConfigFile, clients: &Clients) -> UdcSetupOutput {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    log::info!("⏳ Starting UDC (Universal Deployer Contract) deployment");
    let udc = UdcSetup::new(account.clone(), account.address(), config_file, clients);
    let udc_setup_outputs = udc.setup().await;
    log::info!(
        "*️⃣ UDC setup completed. [UDC Address : {:?}, UDC class hash : {:?}]",
        udc_setup_outputs.udc_address,
        udc_setup_outputs.udc_class_hash
    );
    log::info!("✅ UDC (Universal Deployer Contract) deployment complete.");
    udc_setup_outputs
}

async fn setup_argent(
    account: Option<RpcAccount<'_>>,
    config_file: &ConfigFile,
    clients: &Clients,
) -> ArgentSetupOutput {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    log::info!("⏳ Starting Argent Account deployment");
    let argent = ArgentSetup::new(account.clone());
    let argent_setup_outputs = argent.setup().await;
    log::info!("*️⃣ Argent setup completed. [Argent account class hash : {:?}]", argent_setup_outputs.argent_class_hash);
    log::info!("✅ Argent Account deployment complete.");
    argent_setup_outputs
}

async fn setup_braavos(
    account: Option<RpcAccount<'_>>,
    config_file: &ConfigFile,
    clients: &Clients,
    udc_address: Felt,
) -> BraavosSetupOutput {
    let account = match account {
        Some(account) => account,
        None => get_account(clients, config_file).await,
    };
    log::info!("⏳ Starting Braavos Account deployment");
    let braavos = BraavosSetup::new(account.clone(), config_file, clients, udc_address);
    let braavos_setup_outputs = braavos.setup().await;
    log::info!(
        "*️⃣ Braavos setup completed. [Braavos account class hash : {:?}]",
        braavos_setup_outputs.braavos_class_hash
    );
    log::info!("✅ Braavos Account deployment complete.");
    braavos_setup_outputs
}

pub async fn setup_l2(config_file: &mut ConfigFile, clients: &Clients) -> BootstrapperOutput {
    // Had to create a temporary clone otherwise the `ConfigFile`
    // will be dropped after passing into `get_account` function.
    let config_file_clone = &config_file.clone();
    let account = get_account(clients, config_file_clone).await;

    let core_contract_client = get_core_contract_client(config_file, clients);

    // setup eth bridge
    let eth_bridge_setup_outputs =
        setup_eth_bridge(Some(account.clone()), &core_contract_client, config_file, clients).await;

    // setup erc20 bridge
    let erc20_bridge_setup_outputs =
        setup_erc20_bridge(Some(account.clone()), &core_contract_client, config_file, clients).await;

    // setup udc
    let udc_setup_outputs = setup_udc(Some(account.clone()), config_file, clients).await;

    // setup argent account
    let argent_setup_outputs = setup_argent(Some(account.clone()), config_file, clients).await;

    // setup braavos account
    let braavos_setup_outputs =
        setup_braavos(Some(account.clone()), config_file, clients, udc_setup_outputs.udc_address).await;

    // upgrading the eth bridge
    config_file.l1_eth_bridge_address = Some(format!(
        "0x{}",
        eth_bridge_setup_outputs.l1_bridge_address.encode_hex().trim_start_matches("0x").trim_start_matches('0')
    ));
    config_file.l2_eth_token_proxy_address = Some(eth_bridge_setup_outputs.l2_eth_proxy_address.to_hex_string());
    config_file.l2_eth_bridge_proxy_address =
        Some(eth_bridge_setup_outputs.l2_eth_bridge_proxy_address.to_hex_string());
    upgrade_eth_bridge(Some(account), config_file, clients).await.expect("Unable to upgrade ETH bridge.");

    BootstrapperOutput {
        eth_bridge_setup_outputs: Some(eth_bridge_setup_outputs),
        erc20_bridge_setup_outputs: Some(erc20_bridge_setup_outputs),
        udc_setup_outputs: Some(udc_setup_outputs),
        argent_setup_outputs: Some(argent_setup_outputs),
        braavos_setup_outputs: Some(braavos_setup_outputs),
        ..Default::default()
    }
}
