pub mod factory;
pub mod implementation_contracts;

use crate::setup::base_layer::BaseLayerSetupTrait;
use crate::utils::save_addresses_to_file;
#[allow(unused_imports)]
use alloy::{
    hex::{self, FromHex, ToHexExt},
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{Provider, ProviderBuilder},
    rpc::types::eth::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolValue,
};
use async_trait::async_trait;
use factory::BaseLayerContracts;
use log;
use serde_json;
use std::collections::HashMap;
use factory::{Factory, FactoryDeploy};
use implementation_contracts::{ImplementationContract, IMPLEMENTATION_CONTRACTS_DATA};
use thiserror::Error;
use anyhow::Result;

// Custom error types using thiserror - grouped into broader categories
#[derive(Error, Debug)]
pub enum EthereumSetupError {
    // Network and provider errors
    #[error("Network error: {0}")]
    Network(String),
    
    // Contract deployment errors
    #[error("Contract deployment error: {0}")]
    ContractDeployment(String),
    
    // Contract execution errors
    #[error("Contract execution error: {0}")]
    ContractExecution(String),
    
    // Configuration and data errors
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    // File I/O and serialization errors
    #[error("File I/O error: {0}")]
    FileIo(String),
}

#[allow(dead_code)]
pub struct EthereumSetup {
    rpc_url: String,
    signer: PrivateKeySigner,
    // TODO: Good to have later is to enforce the
    // keys in ImplementationContracts struct.
    // This might be done using a alloy::sol macro.
    implementation_address: HashMap<ImplementationContract, String>,
    core_contract_init_data: Factory::CoreContractInitData,
    addresses_output_path: String,
    base_layer_contracts: Option<BaseLayerContracts>,
}

impl EthereumSetup {
    pub fn new(
        rpc_url: String,
        private_key: String,
        implementation_address: HashMap<ImplementationContract, String>,
        core_contract_init_data: Factory::CoreContractInitData,
        addresses_output_path: &str,
    ) -> Self {
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        Self {
            rpc_url,
            signer,
            implementation_address,
            core_contract_init_data,
            addresses_output_path: addresses_output_path.to_string(),
            base_layer_contracts: None,
        }
    }

    async fn deploy_contract_from_artifact(&self, artifact_path: &str) -> Result<Address, EthereumSetupError> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|e| EthereumSetupError::Network(format!("Invalid RPC URL: {}", e)))?);

        // Read the artifact file
        let artifact_content = std::fs::read_to_string(artifact_path)
            .map_err(|e| EthereumSetupError::FileIo(format!("Failed to read artifact for {}: {}", artifact_path, e)))?;
        let artifact: serde_json::Value = serde_json::from_str(&artifact_content)
            .map_err(|e| EthereumSetupError::FileIo(format!("Failed to parse artifact JSON: {}", e)))?;

        let bytecode = artifact["bytecode"]["object"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("Bytecode not found in artifact".to_string()))?
            .strip_prefix("0x")
            .ok_or(EthereumSetupError::Configuration("Incorrect bytecode format".to_string()))?
            .to_string();

        let deploy_code = hex::decode(bytecode)
            .map_err(|e| EthereumSetupError::Configuration(format!("Failed to decode bytecode: {}", e)))?;

        let deploy_tx = TransactionRequest::default().with_deploy_code(deploy_code);
        let pending_transaction_builder = provider.send_transaction(deploy_tx).await;
        let transaction_receipt = pending_transaction_builder
            .map_err(|e| EthereumSetupError::ContractDeployment(format!("Failed to send deployment transaction for {}: {}", artifact_path, e)))?
            .get_receipt()
            .await;
        let receipt = transaction_receipt
            .map_err(|e| EthereumSetupError::ContractDeployment(format!("Failed to get transaction receipt: {}", e)))?;

        let address = receipt.contract_address
            .ok_or(EthereumSetupError::ContractDeployment("No contract address in receipt".to_string()))?;
        Ok(address)
    }

    /// Save the current class hashes and addresses to a JSON file
    fn save_ethereum_addresses(&self) -> Result<(), EthereumSetupError> {
        let base_layer_addresses = serde_json::json!({
            "implementation_addresses": {
                "coreContract": self.implementation_address.get(&ImplementationContract::CoreContract),
                "manager": self.implementation_address.get(&ImplementationContract::Manager),
                "registry": self.implementation_address.get(&ImplementationContract::Registry),
                "multiBridge": self.implementation_address.get(&ImplementationContract::MultiBridge),
                "ethBridge": self.implementation_address.get(&ImplementationContract::EthBridge),
                "ethBridgeEIC": self.implementation_address.get(&ImplementationContract::EthBridgeEIC),
                "baseLayerFactory": self.implementation_address.get(&ImplementationContract::BaseLayerFactory),
            },
            "addresses": self.base_layer_contracts.as_ref().map(|contracts| {
                serde_json::to_value(contracts).unwrap_or(serde_json::Value::Null)
            }).unwrap_or(serde_json::Value::Null)
        });

        let json_string = serde_json::to_string_pretty(&base_layer_addresses)
            .map_err(|e| EthereumSetupError::FileIo(format!("JSON serialization error: {}", e)))?;
        
        crate::utils::save_addresses_to_file(json_string, &self.addresses_output_path)
            .map_err(|e| EthereumSetupError::FileIo(format!("Failed to save addresses to {}: {}", self.addresses_output_path, e)))?;

        log::info!("Ethereum base layer addresses saved to: {}", self.addresses_output_path);

        Ok(())
    }
}

// Base layer setup trait implementation
#[async_trait]
impl BaseLayerSetupTrait for EthereumSetup {
    async fn init(&mut self) -> Result<()> {
        for contract_info in IMPLEMENTATION_CONTRACTS_DATA {
            let address = self
                .deploy_contract_from_artifact(contract_info.artifact_path)
                .await
                .map_err(|e| EthereumSetupError::ContractDeployment(format!("Failed to deploy {:?}: {}", contract_info.implementation_contract, e)))?;

            log::info!("Deployed {:?} at address: {:?}", contract_info.implementation_contract, address);
            self.implementation_address.insert(contract_info.implementation_contract, address.to_string());
        }

        // Write the addresses to a JSON file
        let addresses_json = serde_json::to_string_pretty(&self.implementation_address)
            .map_err(|e| EthereumSetupError::FileIo(format!("JSON serialization error: {}", e)))?;
        save_addresses_to_file(addresses_json, &self.addresses_output_path)
            .map_err(|e| EthereumSetupError::FileIo(format!("Failed to save addresses: {}", e)))?;

        Ok(())
    }

    async fn setup(&mut self) -> Result<()> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|e| EthereumSetupError::Network(format!("Invalid RPC URL: {}", e)))?);

        // Convert implementation addresses to the required format
        // (to be passed to the factory constructor)
        let implementation_contracts =
            serde_json::from_str(&serde_json::to_string_pretty(&self.implementation_address)
                .map_err(|e| EthereumSetupError::FileIo(format!("JSON serialization error: {}", e)))?)
                .map_err(|e| EthereumSetupError::Configuration(format!("Failed to parse implementation addresses: {}", e)))?;

        // Deploy the factory contract
        let factory_deploy = FactoryDeploy::new(provider, self.signer.address(), implementation_contracts)
            .await
            .map_err(|e| EthereumSetupError::ContractDeployment(format!("Failed to deploy Ethereum Factory: {}", e)))?;
        log::info!("Deployed factory at {:?}", factory_deploy.address());

        self.implementation_address
            .insert(ImplementationContract::BaseLayerFactory, factory_deploy.address().to_string());

        save_addresses_to_file(
            serde_json::to_string_pretty(&self.implementation_address)
                .map_err(|e| EthereumSetupError::FileIo(format!("JSON serialization error: {}", e)))?,
            &self.addresses_output_path,
        )
        .map_err(|e| EthereumSetupError::FileIo(format!("Failed to save addresses: {}", e)))?;

        let base_layer_contracts = factory_deploy
            .setup(self.core_contract_init_data.clone(), self.signer.address(), self.signer.address())
            .await
            .map_err(|e| EthereumSetupError::ContractExecution(format!("Failed to initialize Ethereum Factory: {}", e)))?;

        // Store the base layer contracts for later use
        self.base_layer_contracts = Some(base_layer_contracts);

        // Save the addresses including the deployed base layer contracts
        self.save_ethereum_addresses()?;

        Ok(())
    }

    async fn post_madara_setup(&mut self, madara_addresses_path: &str) -> Result<()> {
        // Read the base layer factory address from addresses.json
        let addresses_content =
            std::fs::read_to_string(&self.addresses_output_path)
                .map_err(|e| EthereumSetupError::FileIo(format!("Failed to read addresses.json: {}", e)))?;
        let addresses: serde_json::Value =
            serde_json::from_str(&addresses_content)
                .map_err(|e| EthereumSetupError::FileIo(format!("Failed to parse addresses.json: {}", e)))?;

        let base_layer_factory_address = addresses["implementation_addresses"]["baseLayerFactory"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("baseLayerFactory address not found in addresses.json".to_string()))?;

        // Read the L2 bridge addresses from madara_addresses.json
        let madara_addresses_content =
            std::fs::read_to_string(madara_addresses_path)
                .map_err(|e| EthereumSetupError::FileIo(format!("Failed to read madara_addresses.json: {}", e)))?;
        let madara_addresses: serde_json::Value =
            serde_json::from_str(&madara_addresses_content)
                .map_err(|e| EthereumSetupError::FileIo(format!("Failed to parse madara_addresses.json: {}", e)))?;

        let l2_eth_bridge_address = madara_addresses["addresses"]["l2_eth_bridge"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("ethTokenBridge address not found in madara_addresses.json".to_string()))?;
        let l2_erc20_bridge_address = madara_addresses["addresses"]["l2_token_bridge"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("tokenBridge address not found in madara_addresses.json".to_string()))?;

        // Read the deployed bridge addresses from addresses.json
        let eth_token_bridge = addresses["addresses"]["ethTokenBridge"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("ethTokenBridge address not found in addresses.json".to_string()))?;
        let token_bridge = addresses["addresses"]["tokenBridge"]
            .as_str()
            .ok_or(EthereumSetupError::Configuration("tokenBridge address not found in addresses.json".to_string()))?;

        // Convert hex addresses to U256 format
        let l2_eth_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_eth_bridge_address.strip_prefix("0x").unwrap_or(l2_eth_bridge_address),
            16,
        )
        .map_err(|e| EthereumSetupError::Configuration(format!("Failed to convert l2EthBridgeAddress to U256: {}", e)))?;

        let l2_erc20_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_erc20_bridge_address.strip_prefix("0x").unwrap_or(l2_erc20_bridge_address),
            16,
        )
        .map_err(|e| EthereumSetupError::Configuration(format!("Failed to convert l2Erc20BridgeAddress to U256: {}", e)))?;

        // Convert string addresses to Address format
        let eth_token_bridge_address =
            eth_token_bridge.parse::<alloy::primitives::Address>()
                .map_err(|e| EthereumSetupError::Configuration(format!("Failed to parse ethTokenBridge address: {}", e)))?;
        let token_bridge_address =
            token_bridge.parse::<alloy::primitives::Address>()
                .map_err(|e| EthereumSetupError::Configuration(format!("Failed to parse tokenBridge address: {}", e)))?;

        // Create a provider and instantiate the factory contract
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().map_err(|e| EthereumSetupError::Network(format!("Invalid RPC URL: {}", e)))?);

        // Create a new factory instance with the deployed address
        let factory_instance = Factory::new(
            base_layer_factory_address
                .parse::<alloy::primitives::Address>()
                .map_err(|e| EthereumSetupError::Configuration(format!("Failed to parse base_layer_factory address: {}", e)))?,
            provider,
        );

        // Call set_l2_bridge on the factory contract
        factory_instance
            .setL2Bridge(l2_eth_bridge_u256, l2_erc20_bridge_u256, eth_token_bridge_address, token_bridge_address)
            .send()
            .await
            .map_err(|e| EthereumSetupError::ContractExecution(format!("Failed to send setL2Bridge transaction: {}", e)))?
            .watch()
            .await
            .map_err(|e| EthereumSetupError::ContractExecution(format!("Failed to watch setL2Bridge transaction: {}", e)))?;

        log::info!("Successfully called set_l2_bridge on factory contract");

        Ok(())
    }
}
