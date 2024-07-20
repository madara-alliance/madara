use std::str::FromStr;
use std::{env, path::Path};

use alloy::signers::wallet::LocalWallet;
use alloy::{network::Ethereum, providers::ProviderBuilder, rpc::client::RpcClient};
use async_trait::async_trait;
use c_kzg::KzgSettings;
use da_client_interface::DaConfig;
use url::Url;
use utils::env_utils::get_env_var_or_panic;

use crate::EthereumDaClient;

#[derive(Clone, Debug)]
pub struct EthereumDaConfig {
    pub rpc_url: String,
    pub memory_pages_contract: String,
    pub private_key: String,
}

#[async_trait]
impl DaConfig<EthereumDaClient> for EthereumDaConfig {
    fn new_from_env() -> Self {
        Self {
            rpc_url: get_env_var_or_panic("ETHEREUM_RPC_URL"),
            memory_pages_contract: get_env_var_or_panic("MEMORY_PAGES_CONTRACT_ADDRESS"),
            private_key: get_env_var_or_panic("PRIVATE_KEY"),
        }
    }
    async fn build_client(&self) -> EthereumDaClient {
        let client =
            RpcClient::new_http(Url::from_str(self.rpc_url.as_str()).expect("Failed to parse ETHEREUM_RPC_URL"));
        let provider = ProviderBuilder::<_, Ethereum>::new().on_client(client);
        let wallet: LocalWallet = env::var("PK").expect("PK must be set").parse().expect("issue while parsing");
        // let wallet: LocalWallet = config.private_key.as_str().parse();
        let trusted_setup = KzgSettings::load_trusted_setup_file(Path::new("./trusted_setup.txt"))
            .expect("issue while loading the trusted setup");
        EthereumDaClient { provider, wallet, trusted_setup }
    }
}
