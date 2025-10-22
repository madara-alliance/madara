pub mod constants;
pub mod error;
pub mod factory;
pub mod implementation_contracts;

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
use factory::BaseLayerContracts;
use factory::{DeployedFactory, Factory};
use implementation_contracts::ImplementationContract;
use log;
use serde_json;
use std::collections::HashMap;
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
            base_layer_factory_address: None,
            core_contract_init_data,
            addresses_output_path: addresses_output_path.to_string(),
            base_layer_contracts: None,
        }
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
            "addresses": self.base_layer_contracts.as_ref().map(|contracts| {
                serde_json::to_value(contracts).unwrap_or(serde_json::Value::Null)
            }).unwrap_or(serde_json::Value::Null)
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

        // Write the addresses to a JSON file
        let addresses_json = serde_json::to_string_pretty(&self.implementation_address)?;
        save_addresses_to_file(addresses_json, &self.addresses_output_path)?;

        Ok(())
    }

    async fn setup(&mut self) -> Result<(), BaseLayerError> {
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
            .map_err(|e| BaseLayerError::FailedToDeployFactory(e))?;
        log::info!("Deployed factory at {:?}", factory_deploy.address());

        self.base_layer_factory_address = Some(factory_deploy.address().to_string());

        save_addresses_to_file(
            serde_json::to_string_pretty(&self.implementation_address)?,
            &self.addresses_output_path,
        )?;

        let base_layer_contracts = factory_deploy
            .setup(self.core_contract_init_data.clone(), self.signer.address(), self.signer.address())
            .await?;

        // Store the base layer contracts for later use
        self.base_layer_contracts = Some(base_layer_contracts);

        // Save the addresses including the deployed base layer contracts
        self.save_ethereum_addresses()?;

        Ok(())
    }

    async fn post_madara_setup(&mut self, madara_addresses_path: &str) -> Result<(), BaseLayerError> {
        // Read the base layer factory address from addresses.json
        let addresses_content = std::fs::read_to_string(&self.addresses_output_path)
            .map_err(|e| BaseLayerError::FailedToReadBaseLayerOutput(e))?;
        let addresses: serde_json::Value = serde_json::from_str(&addresses_content)?;

        let base_layer_factory_address = addresses["implementation_addresses"]["baseLayerFactory"]
            .as_str()
            .ok_or(BaseLayerError::KeyNotFound(".implementation_addresses.baseLayerFactory".to_string()))?;

        // Read the L2 bridge addresses from madara_addresses.json
        let madara_addresses_content =
            std::fs::read_to_string(madara_addresses_path).map_err(|e| BaseLayerError::FailedToReadMadaraOutput(e))?;
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

        Ok(())
    }
}
