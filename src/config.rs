use crate::da_clients::ethereum::config::EthereumDaConfig;
use crate::da_clients::ethereum::EthereumDaClient;
use crate::da_clients::{DaClient, DaConfig};
use crate::database::mongodb::config::MongoDbConfig;
use crate::database::mongodb::MongoDb;
use crate::database::{Database, DatabaseConfig};
use crate::queue::sqs::SqsQueue;
use crate::queue::{init_consumers, QueueProvider};
use crate::utils::env_utils::get_env_var_or_panic;
use dotenvy::dotenv;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
use std::sync::Arc;
use tokio::sync::OnceCell;

pub struct Config {
    starknet_client: Arc<JsonRpcClient<HttpTransport>>,
    da_client: Box<dyn DaClient>,
    database: Box<dyn Database>,
    queue: Box<dyn QueueProvider>,
}

impl Config {
    pub fn starknet_client(&self) -> &Arc<JsonRpcClient<HttpTransport>> {
        &self.starknet_client
    }

    pub fn da_client(&self) -> &Box<dyn DaClient> {
        &self.da_client
    }

    pub fn database(&self) -> &Box<dyn Database> {
        &self.database
    }

    pub fn queue(&self) -> &Box<dyn QueueProvider> {
        &self.queue
    }
}

pub static CONFIG: OnceCell<Config> = OnceCell::const_new();

async fn init_config() -> Config {
    dotenv().ok();

    // init starknet client
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(get_env_var_or_panic("MADARA_RPC_URL").as_str()).expect("Failed to parse URL"),
    ));

    // init database
    let database = Box::new(MongoDb::new(MongoDbConfig::new_from_env()).await);

    // init the queue
    let queue = Box::new(SqsQueue {});

    Config { starknet_client: Arc::new(provider), da_client: build_da_client(), database, queue }
}

pub async fn config() -> &'static Config {
    CONFIG.get_or_init(init_config).await
}

fn build_da_client() -> Box<dyn DaClient + Send + Sync> {
    match get_env_var_or_panic("DA_LAYER").as_str() {
        "ethereum" => {
            let config = EthereumDaConfig::new_from_env();
            return Box::new(EthereumDaClient::from(config));
        }
        _ => panic!("Unsupported DA layer"),
    }
}
