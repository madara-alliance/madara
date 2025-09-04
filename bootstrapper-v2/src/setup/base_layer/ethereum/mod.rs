pub mod factory;

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
use anyhow::Context;
use async_trait::async_trait;
use factory::BaseLayerContracts;
use log;
use serde_json;
use std::collections::HashMap;

use factory::{Factory, FactoryDeploy};

pub static IMPLEMENTATION_CONTRACTS: [&str; 6] =
    ["coreContract", "manager", "registry", "multiBridge", "ethBridge", "ethBridgeEIC"];

#[allow(dead_code)]
pub struct EthereumSetup {
    rpc_url: String,
    signer: PrivateKeySigner,
    // TODO: Good to have later is to enforce the
    // keys in ImplementationContracts struct.
    // This might be done using a alloy::sol macro.
    implementation_address: HashMap<String, String>,
    core_contract_init_data: Factory::CoreContractInitData,
    addresses_output_path: String,
    base_layer_contracts: Option<BaseLayerContracts>,
}

impl EthereumSetup {
    pub fn new(
        rpc_url: String,
        private_key: String,
        implementation_address: HashMap<String, String>,
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

    async fn deploy_contract_from_artifact(&self, artifact_path: &str) -> anyhow::Result<Address> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().expect("Invalid RPC URL"));

        // Read the artifact file
        let artifact_content = std::fs::read_to_string(artifact_path)
            .with_context(|| format!("Failed to read artifact for: {}", artifact_path))?;
        let artifact: serde_json::Value = serde_json::from_str(&artifact_content)?;

        let bytecode = artifact["bytecode"]["object"]
            .as_str()
            .context("Bytecode not found in artifact")?
            .strip_prefix("0x")
            .context("Incorrect bytecode")?
            .to_string();

        let deploy_code = hex::decode(bytecode).context("Failed to decode bytecode")?;

        let deploy_tx = TransactionRequest::default().with_deploy_code(deploy_code);
        let pending_transaction_builder = provider.send_transaction(deploy_tx).await;
        let transaction_receipt = pending_transaction_builder
            .with_context(|| format!("Failed to send deployment transaction {}", artifact_path))?
            .get_receipt()
            .await;
        let receipt = transaction_receipt.context("Failed to get transaction receipt")?;

        let address = receipt.contract_address.context("No contract address in receipt")?;
        Ok(address)
    }

    /// Save the current class hashes and addresses to a JSON file
    fn save_ethereum_addresses(&self) -> anyhow::Result<()> {
        let base_layer_addresses = serde_json::json!({
            "implementation_addresses": {
                "coreContract": self.implementation_address.get("coreContract"),
                "manager": self.implementation_address.get("manager"),
                "registry": self.implementation_address.get("registry"),
                "multiBridge": self.implementation_address.get("multiBridge"),
                "ethBridge": self.implementation_address.get("ethBridge"),
                "ethBridgeEIC": self.implementation_address.get("ethBridgeEIC"),
                "base_layer_factory": self.implementation_address.get("base_layer_factory"),
            },
            "addresses": self.base_layer_contracts.as_ref().map(|contracts| {
                serde_json::to_value(contracts).unwrap_or(serde_json::Value::Null)
            }).unwrap_or(serde_json::Value::Null)
        });

        crate::utils::save_addresses_to_file(
            serde_json::to_string_pretty(&base_layer_addresses)?,
            &self.addresses_output_path,
        )
        .context("Failed to save addresses")?;

        log::info!("Ethereum base layer addresses saved to: {}", self.addresses_output_path);

        Ok(())
    }
}

// Base layer setup trait implementation
#[async_trait]
impl BaseLayerSetupTrait for EthereumSetup {
    async fn init(&mut self) -> anyhow::Result<()> {
        for contract in IMPLEMENTATION_CONTRACTS {
            // let address = self.implementation_address.get(contract).unwrap();
            if contract == "coreContract" {
                let artifact_path = "../build-artifacts/cairo_lang/Starknet.json";
                let address = self
                    .deploy_contract_from_artifact(artifact_path)
                    .await
                    .with_context(|| format!("Failed to deploy {}", contract))?;

                log::info!("Deployed coreContract at address: {:?}", address);
                self.implementation_address.insert(contract.to_string(), address.to_string());
            } else if contract == "ethBridgeEIC" {
                let artifact_path = "./contracts/ethereum/out/configureSingleBridge.sol/ConfigureSingleBridgeEIC.json";
                let address = self
                    .deploy_contract_from_artifact(artifact_path)
                    .await
                    .with_context(|| format!("Failed to deploy {}", contract))?;
                log::info!("Deployed ConfigureSingleBridgeEIC at address: {:?}", address);
                self.implementation_address.insert(contract.to_string(), address.to_string());
            } else {
                let artifact_path = format!("../build-artifacts/starkgate_latest/solidity/{}.json", contract);
                let address = self
                    .deploy_contract_from_artifact(&artifact_path)
                    .await
                    .with_context(|| format!("Failed to deploy {}", contract))?;
                log::info!("Deployed {} at address: {:?}", contract, address);
                self.implementation_address.insert(contract.to_string(), address.to_string());
            }
        }

        // Write the addresses to a JSON file
        let addresses_json = serde_json::to_string_pretty(&self.implementation_address)?;
        save_addresses_to_file(addresses_json, &self.addresses_output_path)?;

        Ok(())
    }

    async fn setup(&mut self) -> anyhow::Result<()> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().expect("Invalid RPC URL"));

        // Convert implementation addresses to the required format
        // (to be passed to the factory constructor)
        let implementations_addresses = serde_json::to_string_pretty(&self.implementation_address)?;
        let implementation_contracts =
            serde_json::from_str(&implementations_addresses).context("Failed to parse implementation addresses")?;

        // Deploy the factory contract
        let factory_deploy = FactoryDeploy::new(provider, self.signer.address(), implementation_contracts)
            .await
            .context("Failed to deploy Ethereum Factory")?;
        log::info!("Deployed factory at {:?}", factory_deploy.address());

        self.implementation_address.insert("base_layer_factory".to_string(), factory_deploy.address().to_string());

        save_addresses_to_file(
            serde_json::to_string_pretty(&self.implementation_address)?,
            &self.addresses_output_path,
        )
        .context("Failed to save addresses")?;

        let base_layer_contracts = factory_deploy
            .setup(self.core_contract_init_data.clone(), self.signer.address(), self.signer.address())
            .await
            .context("Failed to initialize Ethereum Factory")?;

        // Store the base layer contracts for later use
        self.base_layer_contracts = Some(base_layer_contracts);

        // Save the addresses including the deployed base layer contracts
        self.save_ethereum_addresses()?;

        Ok(())
    }

    #[allow(unused_variables)]
    async fn post_madara_setup(&mut self, madara_addresses_path: &str) -> anyhow::Result<()> {
        // Read the base layer factory address from addresses.json
        let addresses_content =
            std::fs::read_to_string(&self.addresses_output_path).context("Failed to read addresses.json")?;
        let addresses: serde_json::Value =
            serde_json::from_str(&addresses_content).context("Failed to parse addresses.json")?;

        let base_layer_factory_address = addresses["implementation_addresses"]["base_layer_factory"]
            .as_str()
            .context("base_layer_factory address not found in addresses.json")?;

        // Read the L2 bridge addresses from madara_addresses.json
        let madara_addresses_content =
            std::fs::read_to_string(madara_addresses_path).context("Failed to read madara_addresses.json")?;
        let madara_addresses: serde_json::Value =
            serde_json::from_str(&madara_addresses_content).context("Failed to parse madara_addresses.json")?;

        let l2_eth_bridge_address = madara_addresses["addresses"]["l2_eth_bridge"]
            .as_str()
            .context("ethTokenBridge address not found in madara_addresses.json")?;
        let l2_erc20_bridge_address = madara_addresses["addresses"]["l2_token_bridge"]
            .as_str()
            .context("tokenBridge address not found in madara_addresses.json")?;

        // Read the deployed bridge addresses from addresses.json
        let eth_token_bridge = addresses["addresses"]["ethTokenBridge"]
            .as_str()
            .context("ethTokenBridge address not found in addresses.json")?;
        let token_bridge = addresses["addresses"]["tokenBridge"]
            .as_str()
            .context("tokenBridge address not found in addresses.json")?;

        // Convert hex addresses to U256 format
        let l2_eth_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_eth_bridge_address.strip_prefix("0x").unwrap_or(l2_eth_bridge_address),
            16,
        )
        .context("Failed to convert l2EthBridgeAddress to U256")?;

        let l2_erc20_bridge_u256 = alloy::primitives::U256::from_str_radix(
            l2_erc20_bridge_address.strip_prefix("0x").unwrap_or(l2_erc20_bridge_address),
            16,
        )
        .context("Failed to convert l2Erc20BridgeAddress to U256")?;

        // Convert string addresses to Address format
        let eth_token_bridge_address =
            eth_token_bridge.parse::<alloy::primitives::Address>().context("Failed to parse ethTokenBridge address")?;
        let token_bridge_address =
            token_bridge.parse::<alloy::primitives::Address>().context("Failed to parse tokenBridge address")?;

        // Create a provider and instantiate the factory contract
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().expect("Invalid RPC URL"));

        // Create a new factory instance with the deployed address
        let factory_instance = Factory::new(
            base_layer_factory_address
                .parse::<alloy::primitives::Address>()
                .context("Failed to parse base_layer_factory address")?,
            provider,
        );

        // Call set_l2_bridge on the factory contract
        factory_instance
            .setL2Bridge(l2_eth_bridge_u256, l2_erc20_bridge_u256, eth_token_bridge_address, token_bridge_address)
            .send()
            .await?
            .watch()
            .await?;

        log::info!("Successfully called set_l2_bridge on factory contract");

        Ok(())
    }
}
