use std::sync::Arc;

use crate::config::{
    build_alert_client, build_da_client, build_prover_service, build_settlement_client, config_force_init, Config,
};
use crate::data_storage::DataStorage;
use da_client_interface::DaClient;
use httpmock::MockServer;

use crate::alerts::Alerts;
use prover_client_interface::ProverClient;
use settlement_client_interface::SettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
use utils::env_utils::get_env_var_or_panic;
use utils::settings::default::DefaultSettingsProvider;

use crate::database::mongodb::config::MongoDbConfig;
use crate::database::mongodb::MongoDb;
use crate::database::{Database, DatabaseConfig};
use crate::queue::sqs::SqsQueue;
use crate::queue::QueueProvider;
use crate::tests::common::{create_sns_arn, create_sqs_queues, drop_database, get_storage_client};

// Inspiration : https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
// TestConfigBuilder allows to heavily customise the global configs based on the test's requirement.
// Eg: We want to mock only the da client and leave rest to be as it is, use mock_da_client.

// TestBuilder for Config
pub struct TestConfigBuilder {
    /// The starknet client to get data from the node
    starknet_client: Option<Arc<JsonRpcClient<HttpTransport>>>,
    /// The DA client to interact with the DA layer
    da_client: Option<Box<dyn DaClient>>,
    /// The service that produces proof and registers it onchain
    prover_client: Option<Box<dyn ProverClient>>,
    /// Settlement client
    settlement_client: Option<Box<dyn SettlementClient>>,
    /// The database client
    database: Option<Box<dyn Database>>,
    /// Queue client
    queue: Option<Box<dyn QueueProvider>>,
    /// Storage client
    storage: Option<Box<dyn DataStorage>>,
    /// Alerts client
    alerts: Option<Box<dyn Alerts>>,
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestConfigBuilder {
    /// Create a new config
    pub fn new() -> TestConfigBuilder {
        TestConfigBuilder {
            starknet_client: None,
            da_client: None,
            prover_client: None,
            settlement_client: None,
            database: None,
            queue: None,
            storage: None,
            alerts: None,
        }
    }

    pub fn mock_da_client(mut self, da_client: Box<dyn DaClient>) -> TestConfigBuilder {
        self.da_client = Some(da_client);
        self
    }

    pub fn mock_db_client(mut self, db_client: Box<dyn Database>) -> TestConfigBuilder {
        self.database = Some(db_client);
        self
    }

    pub fn mock_settlement_client(mut self, settlement_client: Box<dyn SettlementClient>) -> TestConfigBuilder {
        self.settlement_client = Some(settlement_client);
        self
    }

    pub fn mock_starknet_client(mut self, starknet_client: Arc<JsonRpcClient<HttpTransport>>) -> TestConfigBuilder {
        self.starknet_client = Some(starknet_client);
        self
    }

    pub fn mock_prover_client(mut self, prover_client: Box<dyn ProverClient>) -> TestConfigBuilder {
        self.prover_client = Some(prover_client);
        self
    }

    pub fn mock_storage_client(mut self, storage_client: Box<dyn DataStorage>) -> TestConfigBuilder {
        self.storage = Some(storage_client);
        self
    }

    pub fn mock_queue(mut self, queue: Box<dyn QueueProvider>) -> TestConfigBuilder {
        self.queue = Some(queue);
        self
    }

    pub fn mock_alerts(mut self, alerts: Box<dyn Alerts>) -> TestConfigBuilder {
        self.alerts = Some(alerts);
        self
    }

    pub async fn build(mut self) -> MockServer {
        dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");

        let server = MockServer::start();
        let settings_provider = DefaultSettingsProvider {};

        // init database
        if self.database.is_none() {
            self.database = Some(Box::new(MongoDb::new(MongoDbConfig::new_from_env()).await));
        }

        // init the DA client
        if self.da_client.is_none() {
            self.da_client = Some(build_da_client().await);
        }

        // init the Settings client
        if self.settlement_client.is_none() {
            self.settlement_client = Some(build_settlement_client(&settings_provider).await);
        }

        // init the storage client
        if self.storage.is_none() {
            self.storage = Some(get_storage_client().await);
            match get_env_var_or_panic("DATA_STORAGE").as_str() {
                "s3" => self
                    .storage
                    .as_ref()
                    .unwrap()
                    .build_test_bucket(&get_env_var_or_panic("AWS_S3_BUCKET_NAME"))
                    .await
                    .unwrap(),
                _ => panic!("Unsupported Storage Client"),
            }
        }

        if self.alerts.is_none() {
            self.alerts = Some(build_alert_client().await);
        }

        // Deleting and Creating the queues in sqs.
        create_sqs_queues().await.expect("Not able to delete and create the queues.");
        // Deleting the database
        drop_database().await.expect("Unable to drop the database.");
        // Creating the SNS ARN
        create_sns_arn().await.expect("Unable to create the sns arn");

        let config = Config::new(
            self.starknet_client.unwrap_or_else(|| {
                let provider = JsonRpcClient::new(HttpTransport::new(
                    Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
                ));
                Arc::new(provider)
            }),
            self.da_client.unwrap(),
            self.prover_client.unwrap_or_else(|| build_prover_service(&settings_provider)),
            self.settlement_client.unwrap(),
            self.database.unwrap(),
            self.queue.unwrap_or_else(|| Box::new(SqsQueue {})),
            self.storage.unwrap(),
            self.alerts.unwrap(),
        );

        drop_database().await.unwrap();

        config_force_init(config).await;

        server
    }
}
