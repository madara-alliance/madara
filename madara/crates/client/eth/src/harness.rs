use crate::error::Error;
use alloy::node_bindings::{Anvil, AnvilInstance};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use tokio::sync::OnceCell;
use url::Url;

// https://etherscan.io/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81#eventlog
// The txn we are referring to it is here ^
pub const L1_BLOCK_NUMBER: u64 = 20395662;

#[async_trait]
pub trait AnvilInstanceInitializer: Sync + Send + 'static {
    async fn init() -> AnvilInstance;
}
pub struct SharedAnvil<T: AnvilInstanceInitializer> {
    cell: OnceCell<AnvilInstance>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: AnvilInstanceInitializer> SharedAnvil<T> {
    pub const fn new() -> Self {
        Self { cell: OnceCell::const_new(), _marker: std::marker::PhantomData }
    }

    pub async fn get_instance(&'static self) -> &'static AnvilInstance {
        self.cell.get_or_init(T::init).await
    }
}

impl<T: AnvilInstanceInitializer> Default for SharedAnvil<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MainnetFork;
#[async_trait]
impl AnvilInstanceInitializer for MainnetFork {
    async fn init() -> AnvilInstance {
        create_anvil_instance().await.unwrap_or_else(|err| panic!("Error Instantiating Anvil: {:?}", err))
    }
}

pub async fn create_anvil_instance() -> Result<AnvilInstance, Error> {
    let fork_url = std::env::var("ETH_FORK_URL").map_err(|_| Error::MissingForkUrl)?;
    let fork_url = Url::parse(&fork_url).map_err(|_| Error::InvalidForkUrl)?;
    check_endpoint_alive(fork_url.as_str()).await?;

    let instance = Anvil::new()
        .fork(fork_url)
        .fork_block_number(L1_BLOCK_NUMBER)
        .port(0u16)
        // 5 minute start-up timeout to wait for shared resources to become available
        .timeout(300_000)
        .args(["--fork-retry-backoff", "30000"])
        .try_spawn()
        .map_err(Error::NodeSpawnFailed)?;

    Ok(instance)
}

pub async fn check_endpoint_alive(url: &str) -> Result<bool, Error> {
    let client = Client::new();
    let response = client
        .post(url)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "eth_chainId",
            "params": [],
            "id": 1,
        }))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await;

    match response {
        Ok(resp) => {
            if !resp.status().is_success() {
                if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    Err(Error::RpcRateLimited)
                } else {
                    Err(Error::RpcNotResponsive)
                }
            } else {
                Ok(true)
            }
        }
        Err(_) => Err(Error::RpcNotResponsive),
    }
}
