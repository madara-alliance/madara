use crate::setup::base_layer::BaseLayerSetupTrait;
use alloy::network::TransactionBuilder;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::eth::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use anyhow::Context;
use std::collections::HashMap;
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
}

impl EthereumSetup {
    pub fn new(rpc_url: String, private_key: String, implementation_address: HashMap<String, String>) -> Self {
        let signer: PrivateKeySigner = private_key.parse().expect("Failed to parse private key");
        Self { rpc_url, signer, implementation_address }
    }

    async fn deploy_contract_from_artifact(&self, artifact_path: &str) -> anyhow::Result<String> {
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_http(self.rpc_url.parse().expect("Invalid RPC URL"));
        let artifact_content = std::fs::read_to_string(artifact_path)
            .with_context(|| format!("Failed to read artifact for: {}", artifact_path))?;
        let artifact: serde_json::Value = serde_json::from_str(&artifact_content)?;

        let bytecode = artifact["bytecode"]["object"]
            .as_str()
            .context("Bytecode not found in artifact")?
            .strip_prefix("0x")
            .context("Incorrect bytecode")?
            .to_string();

        let deploy_tx = TransactionRequest::default()
            .with_deploy_code(alloy::hex::decode(bytecode).context("Failed to decode bytecode")?);
        let receipt = provider
            .send_transaction(deploy_tx)
            .await
            .context("Failed to send deployment transaction")?
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        let address = receipt.contract_address.context("No contract address in receipt")?;
        Ok(address.to_string())
    }
}

#[allow(unused_variables)]
impl BaseLayerSetupTrait for EthereumSetup {
    fn init(&mut self, addresses_output_path: &str) -> anyhow::Result<()> {
        for contract in IMPLEMENTATION_CONTRACTS {
            // let address = self.implementation_address.get(contract).unwrap();
            let rt = Runtime::new().context("Failed to create runtime")?;

            if contract == "coreContract" {
                let artifact_path = "../build-artifacts/cairo_lang/Starknet.json";
                let address = rt.block_on(self.deploy_contract_from_artifact(artifact_path))?;
                self.implementation_address.insert(contract.to_string(), address);
            } else {
                let artifact_path = format!("../build-artifacts/starkgate_latest/solidity/{}.json", contract);
                let address = rt.block_on(self.deploy_contract_from_artifact(&artifact_path))?;
                self.implementation_address.insert(contract.to_string(), address);
            }
        }
        Ok(())
    }
    fn setup(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn post_madara_setup(&self, base_addresses_path: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
