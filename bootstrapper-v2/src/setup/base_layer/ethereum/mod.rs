pub mod config_hash;
pub mod constants;
pub mod error;
pub mod factory;
pub mod implementation_contracts;

use std::collections::HashMap;

use crate::config::{ConfigHashConfig, CoreContractInitDataPartial};
use crate::setup::base_layer::BaseLayerError;
use crate::setup::base_layer::{ethereum::error::EthereumError, BaseLayerSetupTrait};
use crate::utils::save_addresses_to_file;
use alloy::{
    network::TransactionBuilder,
    primitives::Address,
    providers::{Provider, ProviderBuilder},
    rpc::types::eth::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use anyhow::Result;
use async_trait::async_trait;
use config_hash::ConfigHashParams;
use factory::{BaseLayerContracts, DeployedFactory, Factory, Manager, Starknet};
use implementation_contracts::ImplementationContract;
use log;
use serde_json;
use starknet::core::types::Felt;
use strum::IntoEnumIterator;

#[allow(dead_code)]
pub struct EthereumSetup {
    rpc_url: String,
    signer: PrivateKeySigner,
    // TODO: Good to have later is to enforce the
    // keys in ImplementationContracts struct.
    // This might be done using a alloy::sol macro.
    implementation_address: HashMap<ImplementationContract, String>,
    base_layer_factory_address: Option<String>,
    /// Core contract init data without configHash (computed at runtime)
    core_contract_init_data: CoreContractInitDataPartial,
    /// Configuration for computing the config hash dynamically
    config_hash_config: ConfigHashConfig,
    addresses_output_path: String,
    base_layer_contracts: Option<BaseLayerContracts>,
    /// Deploy mock contracts for testing/anvil
    deploy_test_contracts: bool,
    /// L1 token address (deployed mock or provided)
    l1_token_address: Option<String>,
}

impl EthereumSetup {
    pub fn new(
        rpc_url: String,
        private_key: String,
        implementation_address: HashMap<ImplementationContract, String>,
        core_contract_init_data: CoreContractInitDataPartial,
        config_hash_config: ConfigHashConfig,
        addresses_output_path: &str,
        deploy_test_contracts: bool,
        l1_token_address: Option<String>,
    ) -> Self {
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        Self {
            rpc_url,
            signer,
            implementation_address,
            base_layer_factory_address: None,
            core_contract_init_data,
            config_hash_config,
            addresses_output_path: addresses_output_path.to_string(),
            base_layer_contracts: None,
            deploy_test_contracts,
            l1_token_address,
        }
    }

    /// Computes the config hash using chain_id from config and applying Pedersen hash.
    /// If `fee_token_override` is provided, it overrides the fee token from config.
    fn compute_config_hash(
        &self,
        fee_token_override: Option<Felt>,
    ) -> Result<alloy::primitives::U256, EthereumError> {
        let params = ConfigHashParams::try_from(&self.config_hash_config)?;
        let fee_token = fee_token_override.unwrap_or(params.madara_fee_token);
        let params = params.with_fee_token(fee_token);

        log::info!("Using chain_id: {:#x}, fee_token: {:#x}", params.chain_id, fee_token);

        let config_hash = params.compute_os_config_hash();
        log::info!("Computed config hash: {:#x}", config_hash);

        // Convert Felt to U256 for alloy
        let config_hash_bytes = config_hash.to_bytes_be();
        let config_hash_u256 = alloy::primitives::U256::from_be_slice(&config_hash_bytes);

        Ok(config_hash_u256)
    }

    async fn deploy_contract_from_artifact(&self, artifact_path: &str) -> Result<Address, EthereumError> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?);

        // Read the artifact file
        let artifact_content = std::fs::read_to_string(artifact_path)?;
        let artifact: serde_json::Value = serde_json::from_str(&artifact_content)?;

        let bytecode = artifact["bytecode"]["object"]
            .as_str()
            .ok_or(EthereumError::KeyDoesNotExist("bytecode".to_string()))?
            .strip_prefix("0x")
            .ok_or(EthereumError::InvalidHexValue)?
            .to_string();

        let deploy_code = hex::decode(bytecode)?;

        let deploy_tx = TransactionRequest::default().with_deploy_code(deploy_code);
        let pending_transaction_builder = provider.send_transaction(deploy_tx).await;
        let transaction_receipt = pending_transaction_builder?;
        let receipt = transaction_receipt.get_receipt().await?;

        let address = receipt.contract_address.ok_or(EthereumError::MissingContractInReceipt)?;
        Ok(address)
    }

    /// Save the current class hashes and addresses to a JSON file
    fn save_ethereum_addresses(&self) -> Result<(), EthereumError> {
        // Build the addresses object with base layer contracts and l1Token
        let addresses = match self.base_layer_contracts.as_ref() {
            Some(contracts) => {
                let mut addresses_map =
                    serde_json::to_value(contracts).unwrap_or(serde_json::Value::Object(Default::default()));
                if let serde_json::Value::Object(ref mut map) = addresses_map {
                    if let Some(ref token_addr) = self.l1_token_address {
                        map.insert("l1Token".to_string(), serde_json::Value::String(token_addr.clone()));
                    }
                }
                addresses_map
            }
            None => {
                let mut map = serde_json::Map::new();
                if let Some(ref token_addr) = self.l1_token_address {
                    map.insert("l1Token".to_string(), serde_json::Value::String(token_addr.clone()));
                }
                serde_json::Value::Object(map)
            }
        };

        let base_layer_addresses = serde_json::json!({
            "implementation_addresses": {
                "coreContract": self.implementation_address.get(&ImplementationContract::CoreContract),
                "manager": self.implementation_address.get(&ImplementationContract::Manager),
                "registry": self.implementation_address.get(&ImplementationContract::Registry),
                "multiBridge": self.implementation_address.get(&ImplementationContract::MultiBridge),
                "ethBridge": self.implementation_address.get(&ImplementationContract::EthBridge),
                "ethBridgeEIC": self.implementation_address.get(&ImplementationContract::EthBridgeEIC),
                // TODO: Handle the None case
                "baseLayerFactory": self.base_layer_factory_address,
            },
            "addresses": addresses
        });

        let json_string = serde_json::to_string_pretty(&base_layer_addresses)?;

        crate::utils::save_addresses_to_file(json_string, &self.addresses_output_path)?;

        log::info!("Ethereum base layer addresses saved to: {}", self.addresses_output_path);

        Ok(())
    }

    async fn post_madara(
        &self,
        l2_eth_bridge_address: &str,
        l2_erc20_bridge_address: &str,
        eth_token_bridge: &str,
        token_bridge: &str,
        base_layer_factory_address: &str,
    ) -> Result<(), EthereumError> {
        // Convert hex addresses to U256 format
        let l2_eth_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_eth_bridge_address.strip_prefix("0x").unwrap_or(l2_eth_bridge_address),
            16,
        )?;

        let l2_erc20_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_erc20_bridge_address.strip_prefix("0x").unwrap_or(l2_erc20_bridge_address),
            16,
        )?;

        // Convert string addresses to Address format
        let eth_token_bridge_address = eth_token_bridge.parse::<alloy::primitives::Address>()?;
        let token_bridge_address = token_bridge.parse::<alloy::primitives::Address>()?;

        // Create a provider and instantiate the factory contract
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?);

        // Create a new factory instance with the deployed address
        let factory_instance =
            Factory::new(base_layer_factory_address.parse::<alloy::primitives::Address>()?, provider);

        // Call set_l2_bridge on the factory contract
        factory_instance
            .setL2Bridge(l2_eth_bridge_u256, l2_erc20_bridge_u256, eth_token_bridge_address, token_bridge_address)
            .send()
            .await?
            .watch()
            .await?;

        Ok(())
    }

    /// Enrolls a token bridge on the Manager contract
    async fn enroll_token_bridge(&self, manager_address: &str, l1_token_address: &str) -> Result<(), EthereumError> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?);

        let manager_addr = manager_address.parse::<alloy::primitives::Address>()?;
        let l1_token_addr = l1_token_address.parse::<alloy::primitives::Address>()?;

        let manager_instance = Manager::new(manager_addr, provider);

        // Enrollment fee: must be between MIN_FEE (10^12 on mainnet) and MAX_FEE (10^16)
        // Using 2 × 10^12 wei to be safe
        let enrollment_fee = alloy::primitives::U256::from(2_000_000_000_000u64);

        // Call enrollTokenBridge on the Manager contract
        // The function signature is: enrollTokenBridge(address token) payable
        manager_instance
            .enrollTokenBridge(l1_token_addr)
            .value(enrollment_fee)
            .send()
            .await?
            .watch()
            .await?;

        log::info!(
            "Successfully enrolled token bridge on Manager for L1 token: {} (fee: {} wei)",
            l1_token_address,
            enrollment_fee
        );

        Ok(())
    }
}

// Base layer setup trait implementation
#[async_trait]
impl BaseLayerSetupTrait for EthereumSetup {
    async fn init(&mut self) -> Result<(), BaseLayerError> {
        for contract in ImplementationContract::iter() {
            if self.implementation_address.contains_key(&contract) {
                log::info!(
                    "Skipping deployment of {:?} as it already exists with address: {}",
                    contract,
                    self.implementation_address
                        .get(&contract)
                        .ok_or(EthereumError::KeyDoesNotExist(format!("{:?}", contract)))?
                );
                continue;
            }

            let address = self.deploy_contract_from_artifact(contract.get_artifact_path()).await?;

            log::info!("Deployed {:?} at address: {:?}", contract, address);
            self.implementation_address.insert(contract, address.to_string());
        }

        // Deploy mock token if deploy_test_contracts is enabled
        if self.deploy_test_contracts {
            log::info!("Deploying mock token contract (STRK)...");
            let mock_token_address = self.deploy_contract_from_artifact(constants::MOCK_TOKEN_ARTIFACT).await?;
            log::info!("Deployed mock token at address: {:?}", mock_token_address);
            self.l1_token_address = Some(mock_token_address.to_string());
        }

        // Write the addresses to a JSON file
        let addresses_json = serde_json::to_string_pretty(&self.implementation_address)?;
        save_addresses_to_file(addresses_json, &self.addresses_output_path)?;

        Ok(())
    }

    async fn setup(&mut self) -> Result<(), BaseLayerError> {
        // Compute the config hash using chain_id from config
        let config_hash = self.compute_config_hash(None)?;
        log::info!("Using computed config hash: {:#x}", config_hash);

        // Build the full CoreContractInitData with the computed config hash
        let core_contract_init_data = self.core_contract_init_data.with_config_hash(config_hash);

        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?);

        log::info!("Implementation addresses before serializing: {:?}", self.implementation_address);
        // Convert implementation addresses to the required format
        // (to be passed to the factory constructor)
        let implementation_contracts =
            serde_json::from_str(&serde_json::to_string_pretty(&self.implementation_address)?)?;

        log::info!("Implementation contracts: {:?}", implementation_contracts);

        // Deploy the factory contract
        let factory_deploy = DeployedFactory::deploy_new(provider, self.signer.address(), implementation_contracts)
            .await
            .map_err(BaseLayerError::FailedToDeployFactory)?;
        log::info!("Deployed factory at {:?}", factory_deploy.address());

        self.base_layer_factory_address = Some(factory_deploy.address().to_string());

        save_addresses_to_file(
            serde_json::to_string_pretty(&self.implementation_address)?,
            &self.addresses_output_path,
        )?;

        let base_layer_contracts =
            factory_deploy.setup(core_contract_init_data, self.signer.address(), self.signer.address()).await?;

        // Store the base layer contracts for later use
        self.base_layer_contracts = Some(base_layer_contracts);

        // Save the addresses including the deployed base layer contracts
        self.save_ethereum_addresses()?;

        Ok(())
    }

    async fn post_madara_setup(&mut self, madara_addresses_path: &str) -> Result<(), BaseLayerError> {
        // Read the base layer factory address from addresses.json
        let addresses_content = std::fs::read_to_string(&self.addresses_output_path)
            .map_err(BaseLayerError::FailedToReadBaseLayerOutput)?;
        let addresses: serde_json::Value = serde_json::from_str(&addresses_content)?;

        let base_layer_factory_address = addresses["implementation_addresses"]["baseLayerFactory"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".implementation_addresses.baseLayerFactory".to_string()))?;

        // Read the L2 bridge addresses from madara_addresses.json
        let madara_addresses_content =
            std::fs::read_to_string(madara_addresses_path).map_err(BaseLayerError::FailedToReadMadaraOutput)?;
        let madara_addresses: serde_json::Value = serde_json::from_str(&madara_addresses_content)?;

        let l2_eth_bridge_address = madara_addresses["addresses"]["l2_eth_bridge"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.l2_eth_bridge".to_string()))?;
        let l2_erc20_bridge_address = madara_addresses["addresses"]["l2_token_bridge"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.l2_token_bridge".to_string()))?;

        // Read the deployed bridge addresses from addresses.json
        let eth_token_bridge = addresses["addresses"]["ethTokenBridge"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.ethTokenBridge".to_string()))?;
        let token_bridge = addresses["addresses"]["tokenBridge"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.tokenBridge".to_string()))?;

        self.post_madara(
            l2_eth_bridge_address,
            l2_erc20_bridge_address,
            eth_token_bridge,
            token_bridge,
            base_layer_factory_address,
        )
        .await?;

        log::info!("Successfully called set_l2_bridge on factory contract");

        // Enroll Token Bridge on Manager
        let manager_address = addresses["addresses"]["manager"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.manager".to_string()))?;
        let l1_token_address = addresses["addresses"]["l1Token"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".addresses.l1Token".to_string()))?;

        self.enroll_token_bridge(manager_address, l1_token_address).await?;

        Ok(())
    }

    async fn verify_update_config_hash(
        &self,
        l2_fee_token: &str,
        core_contract_address: &str,
    ) -> Result<(), BaseLayerError> {
        // 1. Parse l2_fee_token to Felt
        let fee_token = Felt::from_hex(l2_fee_token).map_err(|e| {
            EthereumError::FeltParseError(format!("Failed to parse l2_fee_token '{}': {}", l2_fee_token, e))
        })?;

        // 2. Compute expected config hash with this fee token
        let expected_config_hash = self.compute_config_hash(Some(fee_token))?;
        log::info!("Expected config hash (with fee token): {:#x}", expected_config_hash);

        // 3. Call configHash() on CoreContract to get current
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?);

        let core_contract_addr: Address = core_contract_address.parse().map_err(EthereumError::from)?;
        let core_contract = Starknet::new(core_contract_addr, provider.clone());

        let current_config_hash = core_contract.configHash().call().await.map_err(EthereumError::from)?;
        log::info!("Current config hash on CoreContract: {:#x}", current_config_hash);

        // 4. If different, call setConfigHash(expected)
        if current_config_hash != expected_config_hash {
            log::info!("Config hash mismatch detected, updating CoreContract...");

            // Create a new provider with write capabilities
            let write_provider = ProviderBuilder::new().wallet(self.signer.clone()).connect_http(
                self.rpc_url.parse().map_err(|_| EthereumError::RpcUrlParseError(self.rpc_url.to_string()))?,
            );
            let write_core_contract = Starknet::new(core_contract_addr, write_provider);

            write_core_contract
                .setConfigHash(expected_config_hash)
                .send()
                .await
                .map_err(EthereumError::from)?
                .watch()
                .await
                .map_err(EthereumError::from)?;
            log::info!("Config hash updated successfully on CoreContract");
        } else {
            log::info!("Config hash matches, no update needed");
        }

        Ok(())
    }
}
