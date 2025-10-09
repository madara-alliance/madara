pub mod bootstrap_account;
pub mod class_contracts;

use crate::utils::declare_contract;
use crate::{config::MadaraConfig, utils::{execute_v3, get_contract_address_from_deploy_tx, get_contracts_deployed_addresses}};
use anyhow::Context;
use bootstrap_account::BootstrapAccount;
use class_contracts::{MadaraClass, MADARA_CLASSES_DATA};
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
use std::fmt::format;
use std::fs;

// Types for Map keys - using MadaraClass from class_contracts module

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DeployedContract {
    UniversalDeployer,
    MadaraFactory,
    L2EthToken,
    L2EthBridge,
    L2TokenBridge,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
    provider: JsonRpcClient<HttpTransport>,
    account: Option<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>>,
    classes: HashMap<MadaraClass, Felt>,
    addresses: HashMap<DeployedContract, Felt>,
}

#[allow(dead_code)]
impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig) -> anyhow::Result<Self> {
        let provider = JsonRpcClient::new(HttpTransport::new(
            Url::parse(&madara_config.rpc_url).context("Invalid RPC URL format")?
        ));
        Ok(Self {
            rpc_url: madara_config.rpc_url,
            provider,
            account: None,
            classes: HashMap::new(),
            addresses: HashMap::new(),
        })
    }

    pub async fn init(&mut self, private_key: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        // Sierra class artifact. Output of the `starknet-compile` command
        let chain_id = self.provider.chain_id().await.context("Failed to get chain_id")?;
        let bootstrap_account = BootstrapAccount::new(&self.provider, chain_id);

        bootstrap_account.bootstrap_declare().await?;
        self.account = Some(bootstrap_account.deploy_account(private_key).await?);

        log::info!("Starting contract declarations...");

        // Declare all contracts using the data array
        for class_info in MADARA_CLASSES_DATA.iter() {
            log::info!("Declaring contract...");
            let class_hash = declare_contract(
                class_info.sierra_path,
                class_info.casm_path,
                self.account.as_ref().context("Account not initialized. Call init() first.")?,
            )
            .await
            .context(format!("Failed to declare contract: {:?}", class_info.madara_class))?;

            
            log::info!("Contract declared successfully! Class Hash: 0x{:x}", class_hash);
            
            // Store the class hash immediately
            self.insert_class_hash(class_info.madara_class, class_hash);
        }

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
            self.get_class_hash(&MadaraClass::UniversalDeployer).context("Universal deployer not declared")?;

        let calldata = Vec::from([*universal_deployer_class, Felt::ZERO, Felt::ONE, Felt::ZERO]);
        let calls = vec![Call { to: account_address, selector: selector!("deploy_contract"), calldata }];
        let res = execute_v3(account, &calls)
            .await
            .context("Failed to deploy universal_deployer using custom account invoke")?;

        let udc_address = get_contract_address_from_deploy_tx(account_provider, &res)
            .await
            .context("Failed to get contract address from deploy transaction")?;

        self.insert_address(DeployedContract::UniversalDeployer, udc_address);
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
            self.get_class_hash(&MadaraClass::MadaraFactory).context("MadaraFactory class not declared")?;

        let token_bridge_class = self.get_class_hash(&MadaraClass::TokenBridge).context("Token bridge class not declared")?;

        let eic_class = self.get_class_hash(&MadaraClass::Eic).context("EIC class not declared")?;

        let erc20_class = self.get_class_hash(&MadaraClass::Erc20).context("ERC20 class not declared")?;

        // Convert L1 bridge addresses from string to EthAddress
        let l1_eth_bridge_eth = EthAddress::from_hex(l1_eth_bridge_address).context("Invalid L1 ETH bridge address")?;
        let l1_erc20_bridge_eth =
            EthAddress::from_hex(l1_erc20_bridge_address).context("Invalid L1 ERC20 bridge address")?;

        // Create constructor calldata for MadaraFactory
        let constructor_calldata = vec![
            *token_bridge_class,         // token_bridge_class
            *eic_class,                  // eic_class_hash
            *erc20_class,                // erc20_class_hash
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
            *madara_factory_class_hash,                     // class_hash
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

        let madara_factory_address = get_contract_address_from_deploy_tx(account.provider(), &madara_factory_res)
            .await
            .context("Failed to get MadaraFactory contract address from deploy transaction")?;
        self.insert_address(DeployedContract::MadaraFactory, madara_factory_address);

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
        self.insert_address(DeployedContract::L2EthToken, deployed_addresses.l2_eth_token);
        self.insert_address(DeployedContract::L2EthBridge, deployed_addresses.l2_eth_bridge);
        self.insert_address(DeployedContract::L2TokenBridge, deployed_addresses.l2_token_bridge);

        log::info!("Bridge contracts deployment initiated via MadaraFactory");

        Ok(())
    }

    /// Get a class hash by name
    pub fn get_class_hash(&self, name: &MadaraClass) -> Option<&Felt> {
        self.classes.get(name)
    }

    /// Get an address by name
    pub fn get_address(&self, name: &DeployedContract) -> Option<&Felt> {
        self.addresses.get(name)
    }

    /// Insert a class hash
    pub fn insert_class_hash(&mut self, name: MadaraClass, class_hash: Felt) {
        self.classes.insert(name, class_hash);
    }

    /// Insert an address
    pub fn insert_address(&mut self, name: DeployedContract, address: Felt) {
        self.addresses.insert(name, address);
    }

    /// Save the current class hashes and addresses to a JSON file
    fn save_madara_addresses(&self, madara_addresses_path: &str) -> anyhow::Result<()> {
        let madara_addresses = serde_json::json!({
            "classes": {
                "token_bridge": format!("0x{:x}", self.get_class_hash(&MadaraClass::TokenBridge).context("Token bridge class hash not found")?),
                "erc20": format!("0x{:x}", self.get_class_hash(&MadaraClass::Erc20).context("ERC20 class hash not found")?),
                "eic": format!("0x{:x}", self.get_class_hash(&MadaraClass::Eic).context("EIC class hash not found")?),
                "universal_deployer": format!("0x{:x}", self.get_class_hash(&MadaraClass::UniversalDeployer).context("Universal deployer class hash not found")?),
                "madara_factory": format!("0x{:x}", self.get_class_hash(&MadaraClass::MadaraFactory).context("Madara factory class hash not found")?),
            },
            "addresses": {
                "universal_deployer": format!("0x{:x}", self.addresses.get(&DeployedContract::UniversalDeployer).unwrap_or(&Felt::ZERO)),
                "madara_factory": format!("0x{:x}", self.addresses.get(&DeployedContract::MadaraFactory).unwrap_or(&Felt::ZERO)),
                "l2_eth_token": format!("0x{:x}", self.addresses.get(&DeployedContract::L2EthToken).unwrap_or(&Felt::ZERO)),
                "l2_eth_bridge": format!("0x{:x}", self.addresses.get(&DeployedContract::L2EthBridge).unwrap_or(&Felt::ZERO)),
                "l2_token_bridge": format!("0x{:x}", self.addresses.get(&DeployedContract::L2TokenBridge).unwrap_or(&Felt::ZERO)),
            }
        });

        crate::utils::save_addresses_to_file(serde_json::to_string_pretty(&madara_addresses)?, madara_addresses_path)?;

        log::info!("Madara addresses saved to: {}", madara_addresses_path);

        Ok(())
    }
}
