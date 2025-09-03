use crate::setup::base_layer::BaseLayerSetupTrait;
use async_trait::async_trait;
use starknet::core::types::Felt;
use starknet::{
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Url},
    signers::{LocalWallet, SigningKey},
};

#[allow(dead_code)]
#[derive(Debug)]
pub struct StarknetSetup {
    client: LocalWallet,
    provider: JsonRpcClient<HttpTransport>,
}

impl StarknetSetup {
    pub fn new(rpc_url: String, private_key: String) -> Self {
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&rpc_url).expect("Failed to parse RPC URL")));
        let client = LocalWallet::from(SigningKey::from_secret_scalar(
            Felt::from_hex(&private_key).expect("Failed to convert BASE_LAYER_PRIVATE_KEY to Felt"),
        ));
        Self { client, provider }
    }
}

#[allow(unused_variables)]
#[async_trait]
impl BaseLayerSetupTrait for StarknetSetup {
    #[allow(unused_variables)]
    async fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn setup(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn post_madara_setup(&self, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
