#[cfg(feature = "testing")]
use alloy::primitives::Address;
#[cfg(feature = "testing")]
use alloy::providers::RootProvider;
#[cfg(feature = "testing")]
use std::str::FromStr;

use std::sync::Arc;

use aws_config::SdkConfig;
use dotenvy::dotenv;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};

use da_client_interface::DaClient;
use ethereum_settlement_client::EthereumSettlementClient;
use prover_client_interface::ProverClient;
use settlement_client_interface::SettlementClient;
use sharp_service::SharpProverService;
use starknet_settlement_client::StarknetSettlementClient;
use utils::env_utils::get_env_var_or_panic;

use crate::alerts::aws_sns::AWSSNS;
use crate::alerts::Alerts;
use crate::data_storage::aws_s3::AWSS3;
use crate::data_storage::DataStorage;
use aws_config::meta::region::RegionProviderChain;
use aws_config::Region;
use aws_credential_types::Credentials;
use ethereum_da_client::EthereumDaClient;
use utils::settings::env::EnvSettingsProvider;
use utils::settings::Settings;

use crate::database::mongodb::MongoDb;
use crate::database::Database;
use crate::queue::sqs::SqsQueue;
use crate::queue::QueueProvider;

/// The app config. It can be accessed from anywhere inside the service
/// by calling `config` function.
pub struct Config {
    /// The starknet client to get data from the node
    starknet_client: Arc<JsonRpcClient<HttpTransport>>,
    /// The DA client to interact with the DA layer
    da_client: Box<dyn DaClient>,
    /// The service that produces proof and registers it onchain
    prover_client: Box<dyn ProverClient>,
    /// Settlement client
    settlement_client: Box<dyn SettlementClient>,
    /// The database client
    database: Box<dyn Database>,
    /// Queue client
    queue: Box<dyn QueueProvider>,
    /// Storage client
    storage: Box<dyn DataStorage>,
    /// Alerts client
    alerts: Box<dyn Alerts>,
}

/// `ProviderConfig` is an enum used to represent the global config built
/// using the settings provider. More providers can be added eg : GCP, AZURE etc.
///
/// We are using Arc<SdkConfig> because the config size is large and keeping it
/// a pointer is a better way to pass it through.
#[derive(Clone)]
pub enum ProviderConfig {
    AWS(Box<SdkConfig>),
}

impl ProviderConfig {
    pub fn get_aws_client_or_panic(&self) -> &SdkConfig {
        match self {
            ProviderConfig::AWS(config) => config.as_ref(),
        }
    }
}

/// To build a `SdkConfig` for AWS provider.
pub async fn get_aws_config(settings_provider: &impl Settings) -> SdkConfig {
    let region = settings_provider.get_settings_or_panic("AWS_REGION");
    let region_provider = RegionProviderChain::first_try(Region::new(region)).or_default_provider();
    let credentials = Credentials::from_keys(
        settings_provider.get_settings_or_panic("AWS_ACCESS_KEY_ID"),
        settings_provider.get_settings_or_panic("AWS_SECRET_ACCESS_KEY"),
        None,
    );
    aws_config::from_env().credentials_provider(credentials).region(region_provider).load().await
}

/// Initializes the app config
pub async fn init_config() -> Arc<Config> {
    dotenv().ok();

    let settings_provider = EnvSettingsProvider {};
    let provider_config = Arc::new(ProviderConfig::AWS(Box::new(get_aws_config(&settings_provider).await)));

    // init starknet client
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(settings_provider.get_settings_or_panic("MADARA_RPC_URL").as_str()).expect("Failed to parse URL"),
    ));

    // init database
    let database = build_database_client(&settings_provider).await;
    let da_client = build_da_client(&settings_provider).await;
    let settlement_client = build_settlement_client(&settings_provider).await;
    let prover_client = build_prover_service(&settings_provider);
    let storage_client = build_storage_client(&settings_provider, provider_config.clone()).await;
    let alerts_client = build_alert_client(&settings_provider, provider_config.clone()).await;

    // init the queue
    // TODO: we use omniqueue for now which doesn't support loading AWS config
    // from `SdkConfig`. We can later move to using `aws_sdk_sqs`. This would require
    // us stop using the generic omniqueue abstractions for message ack/nack
    let queue = build_queue_client();

    Arc::new(Config::new(
        Arc::new(provider),
        da_client,
        prover_client,
        settlement_client,
        database,
        queue,
        storage_client,
        alerts_client,
    ))
}

impl Config {
    /// Create a new config
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        starknet_client: Arc<JsonRpcClient<HttpTransport>>,
        da_client: Box<dyn DaClient>,
        prover_client: Box<dyn ProverClient>,
        settlement_client: Box<dyn SettlementClient>,
        database: Box<dyn Database>,
        queue: Box<dyn QueueProvider>,
        storage: Box<dyn DataStorage>,
        alerts: Box<dyn Alerts>,
    ) -> Self {
        Self { starknet_client, da_client, prover_client, settlement_client, database, queue, storage, alerts }
    }

    /// Returns the starknet client
    pub fn starknet_client(&self) -> &Arc<JsonRpcClient<HttpTransport>> {
        &self.starknet_client
    }

    /// Returns the DA client
    pub fn da_client(&self) -> &dyn DaClient {
        self.da_client.as_ref()
    }

    /// Returns the proving service
    pub fn prover_client(&self) -> &dyn ProverClient {
        self.prover_client.as_ref()
    }

    /// Returns the settlement client
    pub fn settlement_client(&self) -> &dyn SettlementClient {
        self.settlement_client.as_ref()
    }

    /// Returns the database client
    pub fn database(&self) -> &dyn Database {
        self.database.as_ref()
    }

    /// Returns the queue provider
    pub fn queue(&self) -> &dyn QueueProvider {
        self.queue.as_ref()
    }

    /// Returns the storage provider
    pub fn storage(&self) -> &dyn DataStorage {
        self.storage.as_ref()
    }

    /// Returns the alerts client
    pub fn alerts(&self) -> &dyn Alerts {
        self.alerts.as_ref()
    }
}

/// Builds the DA client based on the environment variable DA_LAYER
pub async fn build_da_client(settings_provider: &impl Settings) -> Box<dyn DaClient + Send + Sync> {
    match get_env_var_or_panic("DA_LAYER").as_str() {
        "ethereum" => Box::new(EthereumDaClient::new_with_settings(settings_provider)),
        _ => panic!("Unsupported DA layer"),
    }
}

/// Builds the prover service based on the environment variable PROVER_SERVICE
pub fn build_prover_service(settings_provider: &impl Settings) -> Box<dyn ProverClient> {
    match get_env_var_or_panic("PROVER_SERVICE").as_str() {
        "sharp" => Box::new(SharpProverService::new_with_settings(settings_provider)),
        _ => panic!("Unsupported prover service"),
    }
}

/// Builds the settlement client depending on the env variable SETTLEMENT_LAYER
pub async fn build_settlement_client(settings_provider: &impl Settings) -> Box<dyn SettlementClient + Send + Sync> {
    match get_env_var_or_panic("SETTLEMENT_LAYER").as_str() {
        "ethereum" => {
            #[cfg(not(feature = "testing"))]
            {
                Box::new(EthereumSettlementClient::new_with_settings(settings_provider))
            }
            #[cfg(feature = "testing")]
            {
                Box::new(EthereumSettlementClient::with_test_settings(
                    RootProvider::new_http(get_env_var_or_panic("SETTLEMENT_RPC_URL").as_str().parse().unwrap()),
                    Address::from_str(&get_env_var_or_panic("DEFAULT_L1_CORE_CONTRACT_ADDRESS")).unwrap(),
                    Url::from_str(get_env_var_or_panic("SETTLEMENT_RPC_URL").as_str()).unwrap(),
                    Some(Address::from_str(get_env_var_or_panic("STARKNET_OPERATOR_ADDRESS").as_str()).unwrap()),
                ))
            }
        }
        "starknet" => Box::new(StarknetSettlementClient::new_with_settings(settings_provider).await),
        _ => panic!("Unsupported Settlement layer"),
    }
}

pub async fn build_storage_client(
    settings_provider: &impl Settings,
    provider_config: Arc<ProviderConfig>,
) -> Box<dyn DataStorage + Send + Sync> {
    match get_env_var_or_panic("DATA_STORAGE").as_str() {
        "s3" => Box::new(AWSS3::new_with_settings(settings_provider, provider_config).await),
        _ => panic!("Unsupported Storage Client"),
    }
}

pub async fn build_alert_client(
    settings_provider: &impl Settings,
    provider_config: Arc<ProviderConfig>,
) -> Box<dyn Alerts + Send + Sync> {
    match get_env_var_or_panic("ALERTS").as_str() {
        "sns" => Box::new(AWSSNS::new_with_settings(settings_provider, provider_config).await),
        _ => panic!("Unsupported Alert Client"),
    }
}

pub fn build_queue_client() -> Box<dyn QueueProvider + Send + Sync> {
    match get_env_var_or_panic("QUEUE_PROVIDER").as_str() {
        "sqs" => Box::new(SqsQueue {}),
        _ => panic!("Unsupported Queue Client"),
    }
}

pub async fn build_database_client(settings_provider: &impl Settings) -> Box<dyn Database + Send + Sync> {
    match get_env_var_or_panic("DATABASE").as_str() {
        "mongodb" => Box::new(MongoDb::new_with_settings(settings_provider).await),
        _ => panic!("Unsupported Database Client"),
    }
}
