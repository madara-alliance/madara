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
use std::collections::HashMap;

use factory::{Factory, FactoryDeploy};
use tokio::runtime::Runtime;

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
}

// Base layer setup trait implementation
impl BaseLayerSetupTrait for EthereumSetup {
    fn init(&mut self) -> anyhow::Result<()> {
        for contract in IMPLEMENTATION_CONTRACTS {
            // let address = self.implementation_address.get(contract).unwrap();
            let rt = Runtime::new().context("Failed to create runtime")?;

            if contract == "coreContract" {
                let artifact_path = "../build-artifacts/cairo_lang/Starknet.json";
                let address = rt.block_on(self.deploy_contract_from_artifact(artifact_path))?;

                println!("Deployed coreContract at address: {:?}", address);
                self.implementation_address.insert(contract.to_string(), address.to_string());
            } else {
                let artifact_path = format!("../build-artifacts/starkgate_latest/solidity/{}.json", contract);
                let address = rt
                    .block_on(self.deploy_contract_from_artifact(&artifact_path))
                    .with_context(|| format!("Failed to deploy {}", contract))?;
                println!("Deployed {} at address: {:?}", contract, address);
                self.implementation_address.insert(contract.to_string(), address.to_string());
            }
        }

        // Write the addresses to a JSON file
        let addresses_json = serde_json::to_string_pretty(&self.implementation_address)?;
        save_addresses_to_file(addresses_json, &self.addresses_output_path)?;

        Ok(())
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        let rt = Runtime::new().context("Failed to create runtime")?;

        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().expect("Invalid RPC URL"));

        // Convert implementation addresses to the required format
        // (to be passed to the factory constructor)
        let implementations_addresses = serde_json::to_string_pretty(&self.implementation_address)?;
        let implementation_contracts =
            serde_json::from_str(&implementations_addresses).context("Failed to parse implementation addresses")?;

        // Deploy the factory contract
        let factory_deploy = rt
            .block_on(FactoryDeploy::new(provider, self.signer.address(), implementation_contracts))
            .context("Failed to deploy Ethereum Factory")?;
        println!("Deployed factory at {:?}", factory_deploy.address());

        self.implementation_address.insert("base_layer_factory".to_string(), factory_deploy.address().to_string());

        save_addresses_to_file(
            serde_json::to_string_pretty(&self.implementation_address)?,
            &self.addresses_output_path,
        )
        .context("Failed to save addresses")?;

        rt.block_on(factory_deploy.setup(
            self.core_contract_init_data.clone(),
            self.signer.address(),
            self.signer.address(),
        ))
        .context("Failed to initialize Ethereum Factory")?;

        Ok(())
    }

    #[allow(unused_variables)]
    fn post_madara_setup(&self, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
