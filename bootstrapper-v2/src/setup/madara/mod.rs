use crate::config::MadaraConfig;
use anyhow::Context;
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
use starknet::{
    providers::Provider,
    signers::{LocalWallet, SigningKey},
};
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
    provider: JsonRpcClient<HttpTransport>,
}

#[allow(unused_variables)]
impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig, _private_key: String) -> Self {
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&madara_config.rpc_url).unwrap()));
        Self { rpc_url: madara_config.rpc_url, provider }
    }

    pub fn init(&self) -> anyhow::Result<()> {
        // Sierra class artifact. Output of the `starknet-compile` command
        let contract_artifact: SierraClass = serde_json::from_reader(
            std::fs::File::open("../build-artifacts/argent/ArgentAccount.sierra.json").unwrap(),
        )
        .context("Failed to read Argent sierra file")?;

        let contract_casm_artifact: CompiledClass =
            serde_json::from_reader(std::fs::File::open("../build-artifacts/argent/ArgentAccount.casm.json").unwrap())
                .context("Failed to read Argent casm file")?;

        // Class hash of the compiled CASM class from the `starknet-sierra-compile` command
        let compiled_class_hash = contract_casm_artifact.class_hash()?;

        let signer = LocalWallet::from(SigningKey::from_secret_scalar(Felt::from_hex("0x1234").unwrap()));

        // A felt representation of the string 'BOOTSTRAP'.
        let address = Felt::from_hex("0x424f4f545354524150").unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();

        let chain_id = runtime.block_on(self.provider.chain_id())?;
        let account = SingleOwnerAccount::new(self.provider.clone(), signer, address, chain_id, ExecutionEncoding::New);

        println!("Account creation successful !!");

        // We need to flatten the ABI into a string first
        let flattened_class = contract_artifact.flatten().unwrap();

        let declaration = account.declare_v3(Arc::new(flattened_class), compiled_class_hash);

        let result = runtime.block_on(declaration.send())?;

        println!("Transaction hash: {:#064x}", result.transaction_hash);
        println!("Class hash: {:#064x}", result.class_hash);

        Ok(())
    }

    pub fn setup(&self, base_addresses_path: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
