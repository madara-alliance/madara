use crate::database::mongodb::config::MongoDbConfig;
use crate::database::mongodb::MongoDb;
use crate::database::{Database, DatabaseConfig};
use crate::queue::sqs::SqsQueue;
use crate::queue::QueueProvider;
use crate::utils::env_utils::get_env_var_or_panic;
use da_client_interface::{DaClient, DaConfig};
use dotenvy::dotenv;
use ethereum_da_client::config::EthereumDaConfig;
use ethereum_da_client::EthereumDaClient;
use mockall::{automock, predicate::*};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{Provider, ProviderError, JsonRpcClient, Url};
use starknet_core::types::{MaybePendingStateUpdate, BlockId};
use std::sync::Arc;
use tokio::sync::OnceCell;
use async_trait::async_trait;

pub struct MockProvider {
    // You can add state here that might be useful for your tests
}

impl MockProvider {
    pub fn new() -> Self {
        MockProvider {
            // Initialize with any necessary state
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn spec_version(&self) -> Result<String, ProviderError> {
        Ok("mock_version".to_string())
    }
    // Mock other methods similarly...
}

pub enum ProviderEnum {
    JsonRpcClient(JsonRpcClient<HttpTransport>),
    MockProvider(MockProvider),  // Mock implementation
}

#[async_trait]
impl Provider for ProviderEnum {
    async fn spec_version(&self) -> Result<String, ProviderError> {
        match self {
            ProviderEnum::JsonRpcClient(client) => client.spec_version().await,
            ProviderEnum::MockProvider(mock) => mock.spec_version().await,
        }
    }
    // Continue implementing other Provider trait methods...
}

#[automock]
pub trait Config: Send + Sync {
    fn starknet_client(&self) -> &ProviderEnum;
    fn da_client(&self) -> &dyn DaClient;
    fn database(&self) -> &dyn Database;
    fn queue(&self) -> &dyn QueueProvider;
}

/// The app config. It can be accessed from anywhere inside the service
/// by calling `config` function.
pub struct AppConfig {
    /// The starknet client to get data from the node
    starknet_client: Box<ProviderEnum>,
    /// The DA client to interact with the DA layer
    da_client: Box<dyn DaClient>,
    /// The database client
    database: Box<dyn Database>,
    /// The queue provider
    queue: Box<dyn QueueProvider>,
}

impl Config for AppConfig {
    /// Returns the starknet client
    fn starknet_client(&self) -> &ProviderEnum {
        &self.starknet_client
    }

    /// Returns the DA client
    fn da_client(&self) -> &dyn DaClient {
        self.da_client.as_ref()
    }

    /// Returns the database client
    fn database(&self) -> &dyn Database {
        self.database.as_ref()
    }

    /// Returns the queue provider
    fn queue(&self) -> &dyn QueueProvider {
        self.queue.as_ref()
    }
}

/// The app config. It can be accessed from anywhere inside the service.
/// It's initialized only once.
pub static CONFIG: OnceCell<AppConfig> = OnceCell::const_new();

/// Initializes mock app config
#[cfg(test)]
async fn init_config() -> AppConfig {
    dotenv().ok();

    let url = Url::parse(&get_env_var_or_panic("MADARA_RPC_URL")).expect("Failed to parse URL");
    // let starknet_provider = Box::new(ProviderEnum::JsonRpcClient(JsonRpcClient::new(HttpTransport::new(url))));
    let starknet_provider = Box::new(ProviderEnum::MockProvider(MockProvider::new()));

    let database = Box::new(MongoDb::new(MongoDbConfig::new_from_env()).await);
    let queue = Box::new(SqsQueue {});

    AppConfig { starknet_client: starknet_provider, da_client: build_da_client(), database, queue }
}

/// Initializes the app config
#[cfg(not(test))]
async fn init_config() -> AppConfig {
    dotenv().ok();

    let url = Url::parse(&get_env_var_or_panic("MADARA_RPC_URL")).expect("Failed to parse URL");
    let starknet_provider = Box::new(ProviderEnum::JsonRpcClient(JsonRpcClient::new(HttpTransport::new(url))));

    let database = Box::new(MongoDb::new(MongoDbConfig::new_from_env()).await);
    let queue = Box::new(SqsQueue {});

    AppConfig { starknet_client: starknet_provider, da_client: build_da_client(), database, queue }
}

/// Returns the app config. Initializes if not already done.
pub async fn config() -> &'static dyn Config {
    CONFIG.get_or_init(init_config).await
}

/// Builds the DA client based on the environment variable DA_LAYER
fn build_da_client() -> Box<dyn DaClient + Send + Sync> {
    match get_env_var_or_panic("DA_LAYER").as_str() {
        "ethereum" => {
            let config = EthereumDaConfig::new_from_env();
            Box::new(EthereumDaClient::from(config))
        }
        _ => panic!("Unsupported DA layer"),
    }
}
