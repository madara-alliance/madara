pub mod bootstrap_account;

use crate::config::MadaraConfig;
use anyhow::Context;
use bootstrap_account::BootstrapAccount;
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
use starknet::{providers::Provider, signers::LocalWallet};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
    provider: JsonRpcClient<HttpTransport>,
    account: Option<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>>,
}

impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig) -> Self {
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&madara_config.rpc_url).unwrap()));
        Self { rpc_url: madara_config.rpc_url, provider, account: None }
    }

    pub async fn init(&mut self, private_key: &str) -> anyhow::Result<()> {
        // Sierra class artifact. Output of the `starknet-compile` command
        let chain_id = self.provider.chain_id().await.context("Failed to get chain_id")?;
        let bootstrap_account = BootstrapAccount::new(&self.provider, chain_id);

        bootstrap_account.bootstrap_declare().await?;
        self.account = Some(bootstrap_account.deploy_account(private_key).await?);

        Ok(())
    }

    pub fn setup(&self, base_addresses_path: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
