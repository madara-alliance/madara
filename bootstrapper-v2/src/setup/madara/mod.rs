pub mod bootstrap_account;

use crate::config::MadaraConfig;
use crate::utils::declare_contract;
use anyhow::Context;
use bootstrap_account::BootstrapAccount;
use log;
use starknet::{
    accounts::ConnectedAccount,
    core::types::{BlockId, BlockTag},
    providers::Provider,
    signers::LocalWallet,
};
#[allow(unused_imports)]
use starknet::{
    accounts::{Account, DeclarationV3, ExecutionEncoding, SingleOwnerAccount},
    core::{
        chain_id,
        types::{
            contract::{CompiledClass, SierraClass},
            Felt,
        },
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Url},
};
use std::collections::HashMap;
use std::fs;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
    provider: JsonRpcClient<HttpTransport>,
    account: Option<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>>,
    classes: HashMap<String, Felt>,
    addresses: HashMap<String, String>,
}

impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig) -> Self {
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&madara_config.rpc_url).unwrap()));
        Self {
            rpc_url: madara_config.rpc_url,
            provider,
            account: None,
            classes: HashMap::new(),
            addresses: HashMap::new(),
        }
    }

    pub async fn init(&mut self, private_key: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        // Sierra class artifact. Output of the `starknet-compile` command
        let chain_id = self.provider.chain_id().await.context("Failed to get chain_id")?;
        let bootstrap_account = BootstrapAccount::new(&self.provider, chain_id);

        bootstrap_account.bootstrap_declare().await?;
        self.account = Some(bootstrap_account.deploy_account(private_key).await?);

        // Declare the required contract classes
        let account = self.account.as_ref().unwrap();

        log::info!("Starting contract declarations...");

        // Declare token bridge contracts
        log::info!("Declaring token bridge contract...");
        let token_bridge_class = declare_contract(
            "../build-artifacts/starkgate_latest/token_bridge.sierra.json",
            "../build-artifacts/starkgate_latest/token_bridge.casm.json",
            account,
        )
        .await;
        self.classes.insert("token_bridge".to_string(), token_bridge_class);
        log::info!("Token bridge contract declared successfully!");

        log::info!("Declaring ERC20 contract...");
        let erc20_class = declare_contract(
            "../build-artifacts/starkgate_latest/erc20.sierra.json",
            "../build-artifacts/starkgate_latest/erc20.casm.json",
            account,
        )
        .await;
        self.classes.insert("erc20".to_string(), erc20_class);
        log::info!("ERC20 contract declared successfully!");

        // Declare EIC contract
        log::info!("Declaring EIC contract...");
        let eic_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_EIC.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_EIC.compiled_contract_class.json",
            account,
        )
        .await;
        self.classes.insert("eic".to_string(), eic_class);
        log::info!("EIC contract declared successfully!");

        // Declare UniversalDeployer contract
        log::info!("Declaring UniversalDeployer contract...");
        let universal_deployer_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.compiled_contract_class.json",
            account,
        )
        .await;
        self.classes.insert("universal_deployer".to_string(), universal_deployer_class);
        log::info!("UniversalDeployer contract declared successfully!");

        // Declare MadaraFactory contract
        log::info!("Declaring MadaraFactory contract...");
        let madara_factory_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.compiled_contract_class.json",
            account,
        )
        .await;
        self.classes.insert("madara_factory".to_string(), madara_factory_class);
        log::info!("MadaraFactory contract declared successfully!");

        log::info!("Token Bridge Class Hash: {}", token_bridge_class);
        log::info!("ERC20 Class Hash: {}", erc20_class);
        log::info!("EIC Class Hash: {}", eic_class);
        log::info!("Universal Deployer Class Hash: {}", universal_deployer_class);
        log::info!("Madara Factory Class Hash: {}", madara_factory_class);

        // Save the declared class hashes to the madara addresses file
        let madara_addresses = serde_json::json!({
            "classes": {
                "token_bridge": format!("0x{:x}", self.classes.get("token_bridge").unwrap()),
                "erc20": format!("0x{:x}", self.classes.get("erc20").unwrap()),
                "eic": format!("0x{:x}", self.classes.get("eic").unwrap()),
                "universal_deployer": format!("0x{:x}", self.classes.get("universal_deployer").unwrap()),
                "madara_factory": format!("0x{:x}", self.classes.get("madara_factory").unwrap()),
            }
        });

        crate::utils::save_addresses_to_file(serde_json::to_string_pretty(&madara_addresses)?, madara_addresses_path)?;

        log::info!("Madara addresses saved to: {}", madara_addresses_path);
        log::info!("All contract declarations completed successfully!");

        Ok(())
    }

    pub async fn setup(&self, base_addresses_path: &str, _madara_addresses_path: &str) -> anyhow::Result<()> {
        // Read base addresses to get L1 bridge addresses
        let base_addresses_content = fs::read_to_string(base_addresses_path)
            .with_context(|| format!("Failed to read base addresses from: {}", base_addresses_path))?;

        let base_addresses: HashMap<String, String> =
            serde_json::from_str(&base_addresses_content).with_context(|| "Failed to parse base addresses JSON")?;

        let l1_eth_bridge_address = base_addresses
            .get("ethBridge")
            .ok_or_else(|| anyhow::anyhow!("ethBridge address not found in base addresses"))?;

        let l1_erc20_bridge_address = base_addresses
            .get("multiBridge")
            .ok_or_else(|| anyhow::anyhow!("multiBridge address not found in base addresses"))?;

        // Convert string addresses to EthAddress
        let l1_eth_bridge = l1_eth_bridge_address.clone();
        let l1_erc20_bridge = l1_erc20_bridge_address.clone();

        // Get the account for deployment
        let _account =
            self.account.as_ref().ok_or_else(|| anyhow::anyhow!("Account not initialized. Call init() first."))?;

        // TODO: Deploy MadaraFactory contract with the declared class hashes
        // This would require implementing the deployment logic similar to how other contracts are deployed
        // For now, we'll just log the addresses we found

        log::info!("L1 ETH Bridge Address: {}", l1_eth_bridge);
        log::info!("L1 ERC20 Bridge Address: {}", l1_erc20_bridge);

        Ok(())
    }

    /// Get a class hash by name
    pub fn get_class(&self, name: &str) -> Option<&Felt> {
        self.classes.get(name)
    }

    /// Get an address by name
    pub fn get_address(&self, name: &str) -> Option<&String> {
        self.addresses.get(name)
    }

    /// Insert a class hash
    pub fn insert_class(&mut self, name: String, class_hash: Felt) {
        self.classes.insert(name, class_hash);
    }

    /// Insert an address
    pub fn insert_address(&mut self, name: String, address: String) {
        self.addresses.insert(name, address);
    }
}
