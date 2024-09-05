use std::sync::Arc;

use aws_config::SdkConfig;
use httpmock::MockServer;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};

use da_client_interface::{DaClient, MockDaClient};
use prover_client_interface::{MockProverClient, ProverClient};
use settlement_client_interface::{MockSettlementClient, SettlementClient};
use utils::env_utils::get_env_var_or_panic;
use utils::settings::default::DefaultSettingsProvider;

use crate::alerts::{Alerts, MockAlerts};
use crate::config::{
    build_alert_client, build_da_client, build_database_client, build_prover_service, build_queue_client,
    build_settlement_client, Config,
};
use crate::data_storage::{DataStorage, MockDataStorage};
use crate::database::{Database, MockDatabase};
use crate::queue::{MockQueueProvider, QueueProvider};
use crate::tests::common::{create_sns_arn, create_sqs_queues, drop_database};

use super::common::get_storage_client;

// Inspiration : https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
// TestConfigBuilder allows to heavily customise the global configs based on the test's requirement.
// Eg: We want to mock only the da client and leave rest to be as it is, use mock_da_client.

pub enum MockType {
    StarknetClient(Arc<JsonRpcClient<HttpTransport>>),
    DaClient(Box<dyn DaClient>),
    ProverClient(Box<dyn ProverClient>),
    SettlementClient(Box<dyn SettlementClient>),

    Alerts(Box<dyn Alerts>),
    Database(Box<dyn Database>),
    Queue(Box<dyn QueueProvider>),
    Storage(Box<dyn DataStorage>),
}

// By default, everything is on Dummy.
pub enum ConfigType {
    Mock(MockType),
    Actual,
    Dummy,
}

impl From<JsonRpcClient<HttpTransport>> for ConfigType {
    fn from(client: JsonRpcClient<HttpTransport>) -> Self {
        ConfigType::Mock(MockType::StarknetClient(Arc::new(client)))
    }
}

macro_rules! impl_mock_from {
    ($($mock_type:ty => $variant:ident),+) => {
        $(
            impl From<$mock_type> for ConfigType {
                fn from(client: $mock_type) -> Self {
                    ConfigType::Mock(MockType::$variant(Box::new(client)))
                }
            }
        )+
    };
}

impl_mock_from! {
    MockProverClient => ProverClient,
    MockDatabase => Database,
    MockDaClient => DaClient,
    MockQueueProvider => Queue,
    MockDataStorage => Storage,
    MockSettlementClient => SettlementClient
}

// TestBuilder for Config
pub struct TestConfigBuilder {
    /// The starknet client to get data from the node
    starknet_client_type: ConfigType,
    /// The DA client to interact with the DA layer
    da_client_type: ConfigType,
    /// The service that produces proof and registers it on chain
    prover_client_type: ConfigType,
    /// Settlement client
    settlement_client_type: ConfigType,

    /// Alerts client
    alerts_type: ConfigType,
    /// The database client
    database_type: ConfigType,
    /// Queue client
    queue_type: ConfigType,
    /// Storage client
    storage_type: ConfigType,
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TestConfigBuilderReturns {
    pub server: Option<MockServer>,
    pub config: Arc<Config>,
    pub aws_config: SdkConfig,
}
impl TestConfigBuilder {
    /// Create a new config
    pub fn new() -> TestConfigBuilder {
        TestConfigBuilder {
            starknet_client_type: ConfigType::Dummy,
            da_client_type: ConfigType::Dummy,
            prover_client_type: ConfigType::Dummy,
            settlement_client_type: ConfigType::Dummy,
            database_type: ConfigType::Dummy,
            queue_type: ConfigType::Dummy,
            storage_type: ConfigType::Dummy,
            alerts_type: ConfigType::Dummy,
        }
    }

    pub fn configure_da_client(mut self, da_client_type: ConfigType) -> TestConfigBuilder {
        self.da_client_type = da_client_type;
        self
    }

    pub fn configure_settlement_client(mut self, settlement_client_type: ConfigType) -> TestConfigBuilder {
        self.settlement_client_type = settlement_client_type;
        self
    }

    pub fn configure_starknet_client(mut self, starknet_client_type: ConfigType) -> TestConfigBuilder {
        self.starknet_client_type = starknet_client_type;
        self
    }

    pub fn configure_prover_client(mut self, prover_client_type: ConfigType) -> TestConfigBuilder {
        self.prover_client_type = prover_client_type;
        self
    }

    pub fn configure_alerts(mut self, alert_option: ConfigType) -> TestConfigBuilder {
        self.alerts_type = alert_option;
        self
    }

    pub fn configure_storage_client(mut self, storage_client_option: ConfigType) -> TestConfigBuilder {
        self.storage_type = storage_client_option;
        self
    }

    pub fn configure_queue_client(mut self, queue_type: ConfigType) -> TestConfigBuilder {
        self.queue_type = queue_type;
        self
    }
    pub fn configure_database(mut self, database_type: ConfigType) -> TestConfigBuilder {
        self.database_type = database_type;
        self
    }

    pub async fn build(self) -> TestConfigBuilderReturns {
        dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");

        let aws_config = aws_config::load_from_env().await;

        use std::sync::Arc;

        let TestConfigBuilder {
            starknet_client_type,
            alerts_type,
            da_client_type,
            prover_client_type,
            settlement_client_type,
            database_type,
            queue_type,
            storage_type,
        } = self;

        let (starknet_client, server) = init_starknet_client(starknet_client_type).await;
        let alerts = init_alerts(alerts_type, &aws_config).await;
        let da_client = init_da_client(da_client_type).await;

        let settlement_client = init_settlement_client(settlement_client_type).await;

        let prover_client = init_prover_client(prover_client_type).await;

        // External Dependencies
        let storage = init_storage_client(storage_type).await;
        let database = init_database(database_type).await;
        let queue = init_queue_client(queue_type).await;
        // Deleting and Creating the queues in sqs.
        create_sqs_queues().await.expect("Not able to delete and create the queues.");
        // Deleting the database
        drop_database().await.expect("Unable to drop the database.");
        // Creating the SNS ARN
        create_sns_arn(&aws_config).await.expect("Unable to create the sns arn");

        let config = Arc::new(Config::new(
            starknet_client,
            da_client,
            prover_client,
            settlement_client,
            database,
            queue,
            storage,
            alerts,
        ));

        TestConfigBuilderReturns { server, config, aws_config }
    }
}

macro_rules! impl_mock_match {
    ($client:expr, $mock_type:path, $client_type:ty) => {
        match $client {
            $mock_type(client) => client,
            _ => panic!(concat!("Mock client is not a ", stringify!($client_type))),
        }
    };
}

async fn init_da_client(service: ConfigType) -> Box<dyn DaClient> {
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::DaClient, DaClient),
        ConfigType::Actual => build_da_client().await,
        ConfigType::Dummy => Box::new(MockDaClient::new()),
    }
}

async fn init_settlement_client(service: ConfigType) -> Box<dyn SettlementClient> {
    let settings_provider = DefaultSettingsProvider {};
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::SettlementClient, SettlementClient),
        ConfigType::Actual => build_settlement_client(&settings_provider).await,
        ConfigType::Dummy => Box::new(MockSettlementClient::new()),
    }
}

async fn init_prover_client(service: ConfigType) -> Box<dyn ProverClient> {
    let settings_provider = DefaultSettingsProvider {};
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::ProverClient, ProverClient),
        ConfigType::Actual => build_prover_service(&settings_provider),
        ConfigType::Dummy => Box::new(MockProverClient::new()),
    }
}

async fn init_alerts(service: ConfigType, aws_config: &SdkConfig) -> Box<dyn Alerts> {
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::Alerts, Alerts),
        ConfigType::Actual => build_alert_client(aws_config).await,
        ConfigType::Dummy => Box::new(MockAlerts::new()),
    }
}

async fn init_storage_client(service: ConfigType) -> Box<dyn DataStorage> {
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::Storage, Storage),
        ConfigType::Actual => {
            let storage = get_storage_client().await;
            match get_env_var_or_panic("DATA_STORAGE").as_str() {
                "s3" => storage.as_ref().build_test_bucket(&get_env_var_or_panic("AWS_S3_BUCKET_NAME")).await.unwrap(),
                _ => panic!("Unsupported Storage Client"),
            }
            storage
        }
        ConfigType::Dummy => Box::new(MockDataStorage::new()),
    }
}

async fn init_queue_client(service: ConfigType) -> Box<dyn QueueProvider> {
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::Queue, Queue),
        ConfigType::Actual => build_queue_client(),
        ConfigType::Dummy => Box::new(MockQueueProvider::new()),
    }
}

async fn init_database(service: ConfigType) -> Box<dyn Database> {
    match service {
        ConfigType::Mock(client) => impl_mock_match!(client, MockType::Database, Database),
        ConfigType::Actual => build_database_client().await,
        ConfigType::Dummy => Box::new(MockDatabase::new()),
    }
}

async fn init_starknet_client(service: ConfigType) -> (Arc<JsonRpcClient<HttpTransport>>, Option<MockServer>) {
    fn get_provider() -> (Arc<JsonRpcClient<HttpTransport>>, Option<MockServer>) {
        let server = MockServer::start();
        let port = server.port();
        let service = Arc::new(JsonRpcClient::new(HttpTransport::new(
            Url::parse(format!("http://localhost:{}", port).as_str()).expect("Failed to parse URL"),
        )));
        (service, Some(server))
    }

    match service {
        ConfigType::Mock(client) => {
            if let MockType::StarknetClient(starknet_client) = client {
                (starknet_client, None)
            } else {
                panic!("Mock client is not a Starknet Client");
            }
        }
        ConfigType::Actual | ConfigType::Dummy => get_provider(),
    }
}
