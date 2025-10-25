use crate::transport::{AdminRPCBypassTransport, AdminRPCProvider};
use crate::ConfigFile;
use ethereum_instance::EthereumClient;
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet::providers::Url;

pub type RpcClientProvider = JsonRpcClient<AdminRPCBypassTransport>;

pub struct Clients {
    eth_client: EthereumClient,
    _provider_l2: JsonRpcClient<HttpTransport>,
    provider_l2_bypass: JsonRpcClient<AdminRPCBypassTransport>,
    l2_admin_rpc: AdminRPCProvider,
}

impl Clients {
    // pub fn provider_l2(&self) -> &JsonRpcClient<HttpTransport> {
    //     &self.provider_l2
    // }

    /// Using this provider, when sending a transaction, it will go through the bypass channel in the admin RPC.
    /// Warning though, transaction validation is disabled in this RPC. This endpoint will not report errors when
    /// transactions are not valid. Check the log for that, they will appear as rejected transactions during
    /// block production.
    pub fn provider_l2(&self) -> &RpcClientProvider {
        &self.provider_l2_bypass
    }

    pub fn l2_admin_rpc(&self) -> &AdminRPCProvider {
        &self.l2_admin_rpc
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

        let provider_l2_transport = HttpTransport::new(
            Url::parse(&config_file.rollup_seq_url).expect("Failed to declare provider for app chain"),
        );
        let l2_admin_rpc = AdminRPCProvider::new(
            Url::parse(&config_file.rollup_declare_v0_seq_url).expect("Failed to parse rollup_declare_v0_seq_url"),
        );
        let provider_l2 = JsonRpcClient::new(provider_l2_transport.clone());

        let provider_l2_bypass =
            JsonRpcClient::new(AdminRPCBypassTransport { inner: provider_l2_transport, admin: l2_admin_rpc.clone() });

        Self { eth_client: client_instance, _provider_l2: provider_l2, provider_l2_bypass, l2_admin_rpc }
    }
}
