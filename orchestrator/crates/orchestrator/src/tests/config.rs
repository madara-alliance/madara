use std::net::SocketAddr;
use std::str::FromStr as _;
use std::sync::Arc;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::core::client::storage::MockStorageClient;
use crate::core::client::AlertClient;
use crate::core::cloud::CloudProvider;
use crate::core::config::{Config, ConfigParam};
use crate::core::{DatabaseClient, QueueClient, StorageClient};
use crate::server::{get_server_url, setup_server};
use crate::tests::common::{create_queues, create_sns_arn, drop_database};
use crate::types::params::cloud_provider::AWSCredentials;
use crate::types::params::da::DAConfig;
use crate::types::params::database::DatabaseArgs;
use crate::types::params::prover::ProverConfig;
use crate::types::params::service::{ServerParams, ServiceParams};
use crate::types::params::settlement::SettlementConfig;
use crate::types::params::snos::SNOSParams;
use crate::types::params::{AlertArgs, OTELConfig, QueueArgs, StorageArgs};
use crate::utils::helpers::ProcessingLocks;
use crate::OrchestratorError;
use alloy::primitives::Address;
use axum::Router;
use cairo_vm::types::layout_name::LayoutName;
use httpmock::MockServer;
use orchestrator_da_client_interface::{DaClient, MockDaClient};
use orchestrator_ethereum_da_client::EthereumDaValidatedArgs;
use orchestrator_ethereum_settlement_client::EthereumSettlementValidatedArgs;
use orchestrator_prover_client_interface::{MockProverClient, ProverClient};
use orchestrator_settlement_client_interface::{MockSettlementClient, SettlementClient};
use orchestrator_sharp_service::SharpValidatedArgs;
use orchestrator_utils::env_utils::{get_env_var_optional, get_env_var_or_panic};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
// Inspiration : https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
// TestConfigBuilder allows to heavily customise the global configs based on the test's requirement.
// Eg: We want to mock only the da client and leave rest to be as it is, use mock_da_client.

pub enum MockType {
    Server(Router),
    RpcUrl(Url),
    StarknetClient(Arc<JsonRpcClient<HttpTransport>>),
    DaClient(Box<dyn DaClient>),
    ProverClient(Box<dyn ProverClient>),
    SettlementClient(Box<dyn SettlementClient>),

    Alerts(Box<dyn AlertClient>),
    Database(Box<dyn DatabaseClient>),
    Queue(Box<dyn QueueClient>),
    Storage(Box<dyn StorageClient>),
}

// By default, everything is on Dummy.
#[derive(Default)]
pub enum ConfigType {
    Mock(MockType),
    Actual,
    #[default]
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
    MockDatabaseClient => Database,
    MockDaClient => DaClient,
    MockQueueClient => Queue,
    MockStorageClient => Storage,
    MockSettlementClient => SettlementClient
}

// TestBuilder for Config
pub struct TestConfigBuilder {
    /// The RPC url used by the starknet client
    starknet_rpc_url_type: ConfigType,
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
    /// API Service
    api_server_type: ConfigType,
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TestConfigBuilderReturns {
    pub starknet_server: Option<MockServer>,
    pub config: Arc<Config>,
    pub provider_config: Arc<CloudProvider>,
    pub api_server_address: Option<SocketAddr>,
}

impl TestConfigBuilder {
    /// Create a new config
    pub fn new() -> TestConfigBuilder {
        TestConfigBuilder {
            starknet_rpc_url_type: ConfigType::default(),
            starknet_client_type: ConfigType::default(),
            da_client_type: ConfigType::default(),
            prover_client_type: ConfigType::default(),
            settlement_client_type: ConfigType::default(),
            database_type: ConfigType::default(),
            queue_type: ConfigType::default(),
            storage_type: ConfigType::default(),
            alerts_type: ConfigType::default(),
            api_server_type: ConfigType::default(),
        }
    }

    pub fn configure_rpc_url(mut self, starknet_rpc_url_type: ConfigType) -> TestConfigBuilder {
        self.starknet_rpc_url_type = starknet_rpc_url_type;
        self
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

    pub fn configure_api_server(mut self, api_server_type: ConfigType) -> TestConfigBuilder {
        self.api_server_type = api_server_type;
        self
    }

    pub async fn build(self) -> TestConfigBuilderReturns {
        dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");

        let params = get_env_params();

        let provider_config =
            Arc::new(CloudProvider::try_from(params.aws_params.clone()).expect("Failed to create provider config"));

        let TestConfigBuilder {
            starknet_rpc_url_type,
            starknet_client_type,
            alerts_type,
            da_client_type,
            prover_client_type,
            settlement_client_type,
            database_type,
            queue_type,
            storage_type,
            api_server_type,
        } = self;

        let (_starknet_rpc_url, starknet_client, starknet_server) =
            implement_client::init_starknet_client(starknet_rpc_url_type, starknet_client_type).await;

        let prover_client = implement_client::init_prover_client(prover_client_type, &params.clone());

        // init alerts
        let alerts = implement_client::init_alerts(alerts_type, &params.alert_params, provider_config.clone()).await;

        let da_client = implement_client::init_da_client(da_client_type, &params.da_params).await;

        let settlement_client =
            implement_client::init_settlement_client(settlement_client_type, &params.settlement_params).await;
        // External Dependencies
        let storage =
            implement_client::init_storage_client(storage_type, &params.storage_params, provider_config.clone()).await;
        let database = implement_client::init_database(database_type, &params.db_params).await;
        let queue =
            implement_client::init_queue_client(queue_type, params.queue_params.clone(), provider_config.clone()).await;
        // Deleting and Creating the queues in sqs.
        create_queues(provider_config.clone(), &params.queue_params)
            .await
            .expect("Not able to delete and create the queues.");
        // Deleting the database
        drop_database(&params.db_params).await.expect("Unable to drop the database.");
        // Creating the SNS ARN
        create_sns_arn(provider_config.clone(), &params.alert_params).await.expect("Unable to create the sns arn");

        let processing_locks = ProcessingLocks::default();

        let config = Arc::new(Config::new(
            params.orchestrator_params,
            starknet_client,
            database,
            storage,
            alerts,
            queue,
            prover_client,
            da_client,
            processing_locks,
            settlement_client,
        ));

        let api_server_address = implement_api_server(api_server_type, config.clone()).await;

        TestConfigBuilderReturns {
            starknet_server,
            config,
            provider_config: provider_config.clone(),
            api_server_address,
        }
    }
}

async fn implement_api_server(api_server_type: ConfigType, config: Arc<Config>) -> Option<SocketAddr> {
    match api_server_type {
        ConfigType::Mock(client) => {
            if let MockType::Server(router) = client {
                let (api_server_url, listener) = get_server_url(config.server_config()).await;
                let app = Router::new().merge(router);

                tokio::spawn(async move {
                    axum::serve(listener, app).await.expect("Failed to start axum server");
                });

                Some(api_server_url)
            } else {
                panic!(concat!("Mock client is not a ", stringify!($client_type)));
            }
        }
        ConfigType::Actual => Some(setup_server(config.clone()).await.expect("Failed to setup server")),
        ConfigType::Dummy => None,
    }
}

pub mod implement_client {
    use std::sync::Arc;

    use httpmock::MockServer;
    use orchestrator_da_client_interface::{DaClient, MockDaClient};
    use orchestrator_prover_client_interface::{MockProverClient, ProverClient};
    use orchestrator_settlement_client_interface::{MockSettlementClient, SettlementClient};
    use starknet::providers::jsonrpc::HttpTransport;
    use starknet::providers::{JsonRpcClient, Url};

    use super::{ConfigType, EnvParams, MockType};
    use crate::core::client::alert::MockAlertClient;
    use crate::core::client::database::MockDatabaseClient;
    use crate::core::client::queue::MockQueueClient;
    use crate::core::client::storage::MockStorageClient;
    use crate::core::client::AlertClient;
    use crate::core::cloud::CloudProvider;
    use crate::core::config::Config;
    use crate::core::traits::resource::Resource;
    use crate::core::{DatabaseClient, QueueClient, StorageClient};
    use crate::tests::common::{delete_storage, get_storage_client};
    use crate::types::params::da::DAConfig;
    use crate::types::params::database::DatabaseArgs;
    use crate::types::params::settlement::SettlementConfig;
    use crate::types::params::{AlertArgs, QueueArgs, StorageArgs};

    macro_rules! implement_mock_client_conversion {
        ($client_type:ident, $mock_variant:ident) => {
            impl From<MockType> for Box<dyn $client_type> {
                fn from(client: MockType) -> Self {
                    if let MockType::$mock_variant(service_client) = client {
                        service_client
                    } else {
                        panic!(concat!("Mock client is not a ", stringify!($client_type)));
                    }
                }
            }
        };
    }

    implement_mock_client_conversion!(StorageClient, Storage);
    implement_mock_client_conversion!(QueueClient, Queue);
    implement_mock_client_conversion!(DatabaseClient, Database);
    implement_mock_client_conversion!(AlertClient, Alerts);
    implement_mock_client_conversion!(ProverClient, ProverClient);
    implement_mock_client_conversion!(SettlementClient, SettlementClient);
    implement_mock_client_conversion!(DaClient, DaClient);

    pub(crate) async fn init_da_client(service: ConfigType, da_params: &DAConfig) -> Box<dyn DaClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => Config::build_da_client(da_params).await,
            ConfigType::Dummy => Box::new(MockDaClient::new()),
        }
    }

    pub(crate) async fn init_settlement_client(
        service: ConfigType,
        settlement_cfg: &SettlementConfig,
    ) -> Box<dyn SettlementClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => {
                Config::build_settlement_client(settlement_cfg).await.expect("Failed to initialise settlement_client")
            }
            ConfigType::Dummy => Box::new(MockSettlementClient::new()),
        }
    }

    pub(crate) fn init_prover_client(service: ConfigType, params: &EnvParams) -> Box<dyn ProverClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => Config::build_prover_service(&params.prover_params, &params.orchestrator_params),
            ConfigType::Dummy => Box::new(MockProverClient::new()),
        }
    }

    pub(crate) async fn init_alerts(
        service: ConfigType,
        alert_params: &AlertArgs,
        provider_config: Arc<CloudProvider>,
    ) -> Box<dyn AlertClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => {
                Config::build_alert_client(alert_params, provider_config).await.expect("error creating alert client")
            }
            ConfigType::Dummy => Box::new(MockAlertClient::new()),
        }
    }

    pub(crate) async fn init_storage_client(
        service: ConfigType,
        storage_cfg: &StorageArgs,
        provider_config: Arc<CloudProvider>,
    ) -> Box<dyn StorageClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => {
                // Delete the Storage before use
                delete_storage(provider_config.clone(), storage_cfg).await.expect("Could not delete storage");
                let storage = get_storage_client(provider_config.clone()).await;
                // First set up the storage
                println!("Setting up the storage , {:?}", storage_cfg);
                storage.setup(storage_cfg.clone()).await.unwrap();
                Config::build_storage_client(storage_cfg, provider_config).await.expect("error creating storage client")
            }
            ConfigType::Dummy => Box::new(MockStorageClient::new()),
        }
    }

    pub(crate) async fn init_queue_client(
        service: ConfigType,
        queue_params: QueueArgs,
        provider_config: Arc<CloudProvider>,
    ) -> Box<dyn QueueClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => {
                Config::build_queue_client(&queue_params, provider_config).await.expect("error creating queue client")
            }
            ConfigType::Dummy => Box::new(MockQueueClient::new()),
        }
    }

    pub(crate) async fn init_database(service: ConfigType, database_params: &DatabaseArgs) -> Box<dyn DatabaseClient> {
        match service {
            ConfigType::Mock(client) => client.into(),
            ConfigType::Actual => {
                Config::build_database_client(database_params).await.expect("error creating database client")
            }
            ConfigType::Dummy => Box::new(MockDatabaseClient::new()),
        }
    }

    pub(crate) async fn init_starknet_client(
        starknet_rpc_url_type: ConfigType,
        service: ConfigType,
    ) -> (Url, Arc<JsonRpcClient<HttpTransport>>, Option<MockServer>) {
        fn get_rpc_url() -> (Url, Option<MockServer>) {
            let server = MockServer::start();
            let port = server.port();
            let rpc_url = Url::parse(format!("http://localhost:{}", port).as_str()).expect("Failed to parse URL");
            (rpc_url, Some(server))
        }

        fn get_provider(rpc_url: &Url) -> Arc<JsonRpcClient<HttpTransport>> {
            Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url.clone())))
        }

        let (rpc_url, server) = match starknet_rpc_url_type {
            ConfigType::Mock(url_type) => {
                if let MockType::RpcUrl(starknet_rpc_url) = url_type {
                    (starknet_rpc_url, None)
                } else {
                    panic!("Mock Rpc URL is not an URL");
                }
            }
            ConfigType::Actual | ConfigType::Dummy => get_rpc_url(),
        };

        let starknet_client = match service {
            ConfigType::Mock(client) => {
                if let MockType::StarknetClient(starknet_client) = client {
                    starknet_client
                } else {
                    panic!("Mock client is not a Starknet Client");
                }
            }
            ConfigType::Actual | ConfigType::Dummy => get_provider(&rpc_url),
        };

        (rpc_url, starknet_client, server)
    }
}

#[derive(Clone)]
pub struct EnvParams {
    aws_params: AWSCredentials,
    alert_params: AlertArgs,
    queue_params: QueueArgs,
    storage_params: StorageArgs,
    db_params: DatabaseArgs,
    da_params: DAConfig,
    settlement_params: SettlementConfig,
    prover_params: ProverConfig,
    orchestrator_params: ConfigParam,
    #[allow(dead_code)]
    instrumentation_params: OTELConfig,
}

pub(crate) fn get_env_params() -> EnvParams {
    let prefix = get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX");
    let db_params = DatabaseArgs {
        connection_uri: get_env_var_or_panic("MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL"),
        database_name: get_env_var_or_panic("MADARA_ORCHESTRATOR_DATABASE_NAME"),
    };

    let storage_params = StorageArgs {
        bucket_name: format!(
            "{}-{}",
            get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX"),
            get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_S3_BUCKET_NAME")
        ),
        bucket_location_constraint: None,
    };

    let queue_params = QueueArgs {
        queue_base_url: get_env_var_or_panic("MADARA_ORCHESTRATOR_SQS_BASE_QUEUE_URL"),
        prefix: prefix.clone(),
        suffix: get_env_var_or_panic("MADARA_ORCHESTRATOR_SQS_SUFFIX"),
    };

    let aws_params = AWSCredentials {
        access_key_id: get_env_var_or_panic("AWS_ACCESS_KEY_ID"),
        secret_access_key: get_env_var_or_panic("AWS_SECRET_ACCESS_KEY"),
        region: get_env_var_or_panic("AWS_REGION"),
    };

    let da_params = DAConfig::Ethereum(EthereumDaValidatedArgs {
        ethereum_da_rpc_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_DA_RPC_URL"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_ETHEREUM_RPC_URL"),
    });

    let arn = get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_SNS_ARN");
    let pos = arn
        .rfind(':')
        .ok_or_else(|| OrchestratorError::SetupCommandError("Invalid ARN format".to_string()))
        .expect("error");
    let sns_arn = format!("{}:{}_{}", &arn[..pos], prefix, &arn[pos + 1..]);

    let alert_params = AlertArgs { endpoint: sns_arn };

    let settlement_params = SettlementConfig::Ethereum(EthereumSettlementValidatedArgs {
        ethereum_rpc_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_ETHEREUM_RPC_URL"),
        ethereum_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY"),
        l1_core_contract_address: Address::from_str(&get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS",
        ))
        .expect("Invalid L1 core contract address"),
        starknet_operator_address: Address::from_str(&get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS",
        ))
        .expect("Invalid Starknet operator address"),
    });

    let snos_config = SNOSParams {
        rpc_for_snos: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_RPC_FOR_SNOS"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_RPC_FOR_SNOS"),
    };

    let env = get_env_var_optional("MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS").expect("Couldn't get max block");
    let max_block: Option<u64> = env.and_then(|s| if s.is_empty() { None } else { Some(s.parse::<u64>().unwrap()) });

    let env = get_env_var_optional("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS").expect("Couldn't get min block");
    let min_block: Option<u64> = env.and_then(|s| if s.is_empty() { None } else { Some(s.parse::<u64>().unwrap()) });

    let env = get_env_var_optional("MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS")
        .expect("Couldn't get max concurrent snos jobs");
    let max_concurrent_snos_jobs: Option<usize> =
        env.and_then(|s| if s.is_empty() { None } else { Some(s.parse::<usize>().unwrap()) });

    let env = get_env_var_optional("MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS")
        .expect("Couldn't get max concurrent proving jobs");
    let max_concurrent_proving_jobs: Option<usize> =
        env.and_then(|s| if s.is_empty() { None } else { Some(s.parse::<usize>().unwrap()) });

    let service_config = ServiceParams {
        max_block_to_process: max_block,
        min_block_to_process: min_block,
        max_concurrent_snos_jobs,
        max_concurrent_proving_jobs,
    };

    let server_config = ServerParams {
        host: get_env_var_or_panic("MADARA_ORCHESTRATOR_HOST"),
        port: get_env_var_or_panic("MADARA_ORCHESTRATOR_PORT")
            .parse()
            .expect("Failed to parse MADARA_ORCHESTRATOR_PORT"),
    };

    let orchestrator_params = ConfigParam {
        madara_rpc_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_MADARA_RPC_URL"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_MADARA_RPC_URL"),
        snos_config,
        service_config,
        server_config,
        snos_layout_name: LayoutName::all_cairo,
        prover_layout_name: LayoutName::dynamic,
    };

    let instrumentation_params = OTELConfig {
        endpoint: get_env_var_optional("MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT")
            .expect("Couldn't get otel collector endpoint")
            .map(|url| Url::parse(&url).expect("Failed to parse MADARA_ORCHESTRATOR_OTEL_COLLECTOR_ENDPOINT")),
        service_name: get_env_var_or_panic("MADARA_ORCHESTRATOR_OTEL_SERVICE_NAME"),
    };

    let prover_params = ProverConfig::Sharp(SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_SHARP_URL"),
        sharp_user_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL"))
            .expect("Failed to parse MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL"),
        sharp_server_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        sharp_proof_layout: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
    });

    EnvParams {
        aws_params,
        alert_params,
        queue_params,
        storage_params,
        db_params,
        da_params,
        settlement_params,
        prover_params,
        instrumentation_params,
        orchestrator_params,
    }
}
