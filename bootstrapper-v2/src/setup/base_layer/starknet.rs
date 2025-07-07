use crate::setup::base_layer::BaseLayerSetupTrait;
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
        let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(&rpc_url).unwrap()));
        let client = LocalWallet::from(SigningKey::from_secret_scalar(
            Felt::from_hex(&private_key).expect("Failed to convert BASE_LAYER_PRIVATE_KEY to Felt"),
        ));
        Self { client, provider }
    }
}

impl BaseLayerSetupTrait for StarknetSetup {
    fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn setup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn post_madara_setup(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
