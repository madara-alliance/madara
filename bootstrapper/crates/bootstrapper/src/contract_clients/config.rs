use ethereum_instance::EthereumClient;
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::providers::Url;

use crate::ConfigFile;

pub struct Clients {
    eth_client: EthereumClient,
    provider_l2: JsonRpcClient<HttpTransport>,
}

impl Clients {
    pub fn provider_l2(&self) -> &JsonRpcClient<HttpTransport> {
        &self.provider_l2
    }

    pub fn eth_client(&self) -> &EthereumClient {
        &self.eth_client
    }

    // To deploy the instance of ethereum and starknet and returning the struct.
    // pub async fn init(config: &CliArgs) -> Self {
    //     let client_instance = EthereumClient::attach(
    //         Option::from(config.eth_rpc.clone()),
    //         Option::from(config.eth_priv_key.clone()),
    //         Option::from(config.eth_chain_id),
    //     )
    //     .unwrap();
    //
    //     let provider_l2 = JsonRpcClient::new(HttpTransport::new(
    //         Url::parse(&config.rollup_seq_url).expect("Failed to declare provider for app chain"),
    //     ));
    //
    //     Self { eth_client: client_instance, provider_l2 }
    // }

    pub async fn init_from_config(config_file: &ConfigFile) -> Self {
        let client_instance = EthereumClient::attach(
            Option::from(config_file.eth_rpc.clone()),
            Option::from(config_file.eth_priv_key.clone()),
            Option::from(config_file.eth_chain_id),
        )
        .unwrap();

        let provider_l2 = JsonRpcClient::new(HttpTransport::new(
            Url::parse(&config_file.rollup_seq_url).expect("Failed to declare provider for app chain"),
        ));

        Self { eth_client: client_instance, provider_l2 }
    }
}
