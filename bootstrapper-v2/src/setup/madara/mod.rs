pub mod account_helpers;
pub mod bootstrap_account;

use crate::utils::declare_contract;
use crate::{config::MadaraConfig, utils::execute_v3};
use account_helpers::{get_contract_address_from_deploy_tx, get_contracts_deployed_addresses};
use anyhow::Context;
use bootstrap_account::BootstrapAccount;
use log;
use starknet::core::types::Call;
use starknet::macros::selector;
#[allow(unused_imports)]
use starknet::{
    accounts::{Account, ConnectedAccount, DeclarationV3, ExecutionEncoding, SingleOwnerAccount},
    contract::ContractFactory,
    core::{
        chain_id,
        types::{
            contract::{CompiledClass, SierraClass},
            EthAddress, Felt,
        },
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Url},
};
use starknet::{providers::Provider, signers::LocalWallet};
use std::collections::HashMap;
use std::fs;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
    provider: JsonRpcClient<HttpTransport>,
    account: Option<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>>,
    classes: HashMap<String, Felt>,
    addresses: HashMap<String, Felt>,
}

#[allow(dead_code)]
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
            "../build-artifacts/starkgate_latest/cairo/token_bridge.sierra.json",
            "../build-artifacts/starkgate_latest/cairo/token_bridge.casm.json",
            account,
        )
        .await;
        let token_bridge_class_hash = token_bridge_class;
        log::info!("Token bridge contract declared successfully!");

        log::info!("Declaring ERC20 contract...");
        let erc20_class = declare_contract(
            "../build-artifacts/starkgate_latest/cairo/ERC20_070.sierra.json",
            "../build-artifacts/starkgate_latest/cairo/ERC20_070.casm.json",
            account,
        )
        .await;
        let erc20_class_hash = erc20_class;
        log::info!("ERC20 contract declared successfully!");

        // Declare EIC contract
        log::info!("Declaring EIC contract...");
        let eic_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_EIC.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_EIC.compiled_contract_class.json",
            account,
        )
        .await;
        let eic_class_hash = eic_class;
        log::info!("EIC contract declared successfully!");

        // Declare UniversalDeployer contract
        log::info!("Declaring UniversalDeployer contract...");
        let universal_deployer_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.compiled_contract_class.json",
            account,
        )
        .await;
        let universal_deployer_class_hash = universal_deployer_class;
        log::info!("Universal Deployer contract declared successfully!");

        // Declare MadaraFactory contract
        log::info!("Declaring MadaraFactory contract...");
        let madara_factory_class = declare_contract(
            "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.contract_class.json",
            "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.compiled_contract_class.json",
            account,
        )
        .await;
        let madara_factory_class_hash = madara_factory_class;
        log::info!("MadaraFactory contract declared successfully!");

        log::info!("Token Bridge Class Hash: 0x{:x}", token_bridge_class_hash);
        log::info!("ERC20 Class Hash: 0x{:x}", erc20_class_hash);
        log::info!("EIC Class Hash: 0x{:x}", eic_class_hash);
        log::info!("Universal Deployer Class Hash: 0x{:x}", universal_deployer_class_hash);
        log::info!("Madara Factory Class Hash: 0x{:x}", madara_factory_class_hash);

        // Store all class hashes after declarations are complete
        self.insert_class_hash("token_bridge", token_bridge_class_hash);
        self.insert_class_hash("erc20", erc20_class_hash);
        self.insert_class_hash("eic", eic_class_hash);
        self.insert_class_hash("universal_deployer", universal_deployer_class_hash);
        self.insert_class_hash("madara_factory", madara_factory_class_hash);

        log::info!("All contract declarations completed successfully!");

        // Save addresses after declarations
        self.save_madara_addresses(madara_addresses_path)?;

        Ok(())
    }

    pub async fn setup(&mut self, base_addresses_path: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        // Read base addresses to get L1 bridge addresses
        let base_addresses_content = fs::read_to_string(base_addresses_path)
            .with_context(|| format!("Failed to read base addresses from: {}", base_addresses_path))?;

        let base_addresses: serde_json::Value =
            serde_json::from_str(&base_addresses_content).with_context(|| "Failed to parse base addresses JSON")?;

        let l1_eth_bridge_address = base_addresses["addresses"]["ethTokenBridge"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("ethTokenBridge address not found in base addresses"))?;

        let l1_erc20_bridge_address = base_addresses["addresses"]["tokenBridge"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("tokenBridge address not found in base addresses"))?;

        log::info!("L1 ETH Bridge Address: {}", l1_eth_bridge_address);
        log::info!("L1 ERC20 Bridge Address: {}", l1_erc20_bridge_address);

        // Deploy universal deployer first
        let udc_address = self.deploy_universal_deployer().await?;

        // Deploy MadaraFactory contract
        let madara_factory_address =
            self.deploy_madara_factory(udc_address, l1_eth_bridge_address, l1_erc20_bridge_address).await?;

        log::info!("MadaraFactory deployed successfully at address: 0x{:x}", madara_factory_address);

        // Call MadaraFactory.deploy_bridges() to deploy bridge contracts
        self.deploy_bridges_via_madara_factory(madara_factory_address).await?;

        // Save addresses after deployment
        self.save_madara_addresses(madara_addresses_path)?;

        Ok(())
    }

    async fn deploy_universal_deployer(&mut self) -> anyhow::Result<Felt> {
        let account =
            self.account.as_ref().ok_or_else(|| anyhow::anyhow!("Account not initialized. Call init() first."))?;

        let account_address = account.address();
        let account_provider = account.provider();

        let universal_deployer_class =
            self.get_class_hash("universal_deployer").context("Universal deployer not declared").unwrap();

        let calldata = Vec::from([*universal_deployer_class, Felt::ZERO, Felt::ONE, Felt::ZERO]);
        let calls = vec![Call { to: account_address, selector: selector!("deploy_contract"), calldata }];
        let res = execute_v3(account, &calls)
            .await
            .context("Failed to deploy universal_deployer using custom account invoke")?;

        let udc_address = get_contract_address_from_deploy_tx(account_provider, &res).await.unwrap();

        self.insert_address("universal_deployer", udc_address);
        log::info!("Universal deployer deployed successfully at address: 0x{:x}", udc_address);

        Ok(udc_address)
    }

    async fn deploy_madara_factory(
        &mut self,
        udc_address: Felt,
        l1_eth_bridge_address: &str,
        l1_erc20_bridge_address: &str,
    ) -> anyhow::Result<Felt> {
        let account =
            self.account.as_ref().ok_or_else(|| anyhow::anyhow!("Account not initialized. Call init() first."))?;

        let account_address = account.address();
        let _account_provider = account.provider();

        log::info!("Deploying MadaraFactory contract...");

        let madara_factory_class_hash =
            *self.get_class_hash("madara_factory").context("MadaraFactory class not declared")?;

        let token_bridge_class = *self.get_class_hash("token_bridge").context("Token bridge class not declared")?;

        let eic_class = *self.get_class_hash("eic").context("EIC class not declared")?;

        let erc20_class = *self.get_class_hash("erc20").context("ERC20 class not declared")?;

        // Convert L1 bridge addresses from string to EthAddress
        let l1_eth_bridge_eth = EthAddress::from_hex(l1_eth_bridge_address).context("Invalid L1 ETH bridge address")?;
        let l1_erc20_bridge_eth =
            EthAddress::from_hex(l1_erc20_bridge_address).context("Invalid L1 ERC20 bridge address")?;

        // Create constructor calldata for MadaraFactory
        let constructor_calldata = vec![
            token_bridge_class,         // token_bridge_class
            eic_class,                  // eic_class_hash
            erc20_class,                // erc20_class_hash
            l1_eth_bridge_eth.into(),   // l1_eth_bridge_address
            l1_erc20_bridge_eth.into(), // l1_erc20_bridge_address
            account_address,            // initial_owner (using the current account address)
        ];

        // TODO: Uncomment this when we can bump starknet.rs, Currently it
        // assumes the deployContract selector instead of the deploy_contract selector
        // let factory = ContractFactory::new_with_udc(madara_factory_class_hash, account, udc_address);
        // let deployment_v3 = factory.deploy_v3(constructor_calldata, Felt::ZERO, false);
        // let madara_factory_address = deployment_v3.deployed_address();

        // let result = deployment_v3.gas(0).gas_price(0).send().await.context("Failed to deploy madara factory")?;
        // wait_for_transaction(&self.provider, result.transaction_hash, "MadaraFactory Deployment").await?;

        // Deploy MadaraFactory using the same pattern as universal deployer
        // The deploy_contract function expects: class_hash, salt, from_zero, calldata
        let mut madara_factory_calldata = vec![
            madara_factory_class_hash,                     // class_hash
            Felt::ZERO,                                    // salt
            Felt::ZERO,                                    // from_zero (false = 0)
            Felt::from(constructor_calldata.len() as u64), // calldata length
        ];
        madara_factory_calldata.extend(constructor_calldata); // append constructor calldata

        let madara_factory_calls =
            vec![Call { to: udc_address, selector: selector!("deploy_contract"), calldata: madara_factory_calldata }];

        let madara_factory_res = execute_v3(account, &madara_factory_calls)
            .await
            .context("Failed to deploy MadaraFactory contract using UDC")?;

        let madara_factory_address =
            get_contract_address_from_deploy_tx(account.provider(), &madara_factory_res).await.unwrap();
        self.insert_address("madara_factory", madara_factory_address);

        log::info!("MadaraFactory deployed successfully at address: 0x{:x}", madara_factory_address);

        Ok(madara_factory_address)
    }

    async fn deploy_bridges_via_madara_factory(&mut self, madara_factory_address: Felt) -> anyhow::Result<()> {
        let account =
            self.account.as_mut().ok_or_else(|| anyhow::anyhow!("Account not initialized. Call init() first."))?;

        log::info!("Calling MadaraFactory.deploy_bridges() to deploy bridge contracts...");

        let deploy_bridges_calls = vec![Call {
            to: madara_factory_address,
            selector: selector!("deploy_bridges"),
            calldata: vec![], // deploy_bridges takes no parameters
        }];

        let deploy_bridges_res = execute_v3(account, &deploy_bridges_calls)
            .await
            .context("Failed to call MadaraFactory.deploy_bridges()")?;

        log::info!(
            "MadaraFactory.deploy_bridges() called successfully. Transaction hash: 0x{:x}",
            deploy_bridges_res.transaction_hash
        );

        // Note: The deploy_bridges function returns (l2_eth_token, l2_eth_bridge, l2_token_bridge)
        // but we need to extract these from events or transaction receipt
        let deployed_addresses = get_contracts_deployed_addresses(account.provider(), &deploy_bridges_res).await?;
        self.insert_address("l2_eth_token", deployed_addresses.l2_eth_token);
        self.insert_address("l2_eth_bridge", deployed_addresses.l2_eth_bridge);
        self.insert_address("l2_token_bridge", deployed_addresses.l2_token_bridge);

        log::info!("Bridge contracts deployment initiated via MadaraFactory");

        Ok(())
    }

    /// Get a class hash by name
    pub fn get_class_hash(&self, name: &str) -> Option<&Felt> {
        self.classes.get(name)
    }

    /// Get an address by name
    pub fn get_address(&self, name: &str) -> Option<&Felt> {
        self.addresses.get(name)
    }

    /// Insert a class hash
    pub fn insert_class_hash(&mut self, name: &str, class_hash: Felt) {
        self.classes.insert(name.to_string(), class_hash);
    }

    /// Insert an address
    pub fn insert_address(&mut self, name: &str, address: Felt) {
        self.addresses.insert(name.to_string(), address);
    }

    /// Save the current class hashes and addresses to a JSON file
    fn save_madara_addresses(&self, madara_addresses_path: &str) -> anyhow::Result<()> {
        let madara_addresses = serde_json::json!({
            "classes": {
                "token_bridge": format!("0x{:x}", self.get_class_hash("token_bridge").unwrap()),
                "erc20": format!("0x{:x}", self.get_class_hash("erc20").unwrap()),
                "eic": format!("0x{:x}", self.get_class_hash("eic").unwrap()),
                "universal_deployer": format!("0x{:x}", self.get_class_hash("universal_deployer").unwrap()),
                "madara_factory": format!("0x{:x}", self.get_class_hash("madara_factory").unwrap()),
            },
            "addresses": {
                "universal_deployer": format!("0x{:x}", self.addresses.get("universal_deployer").unwrap_or(&Felt::ZERO)),
                "madara_factory": format!("0x{:x}", self.addresses.get("madara_factory").unwrap_or(&Felt::ZERO)),
                "l2_eth_token": format!("0x{:x}", self.addresses.get("l2_eth_token").unwrap_or(&Felt::ZERO)),
                "l2_eth_bridge": format!("0x{:x}", self.addresses.get("l2_eth_bridge").unwrap_or(&Felt::ZERO)),
                "l2_token_bridge": format!("0x{:x}", self.addresses.get("l2_token_bridge").unwrap_or(&Felt::ZERO)),
            }
        });

        crate::utils::save_addresses_to_file(serde_json::to_string_pretty(&madara_addresses)?, madara_addresses_path)?;

        log::info!("Madara addresses saved to: {}", madara_addresses_path);

        Ok(())
    }
}
