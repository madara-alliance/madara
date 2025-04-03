use std::sync::Arc;

#[cfg(feature = "testing")]
use alloy::providers::RootProvider;
use aws_config::meta::region::RegionProviderChain;
use aws_config::{Region, SdkConfig};
use aws_credential_types::Credentials;
use cairo_vm::types::layout_name::LayoutName;
use color_eyre::eyre::eyre;
use dotenvy::dotenv;
use orchestrator_atlantic_service::AtlanticProverService;
use orchestrator_da_client_interface::DaClient;
use orchestrator_ethereum_da_client::EthereumDaClient;
use orchestrator_ethereum_settlement_client::EthereumSettlementClient;
use orchestrator_prover_client_interface::ProverClient;
use orchestrator_settlement_client_interface::SettlementClient;
use orchestrator_sharp_service::SharpProverService;
use orchestrator_starknet_settlement_client::StarknetSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};

use crate::alerts::aws_sns::AWSSNS;
use crate::alerts::Alerts;
use crate::cli::alert::AlertValidatedArgs;
use crate::cli::da::DaValidatedArgs;
use crate::cli::database::DatabaseValidatedArgs;
use crate::cli::prover::ProverValidatedArgs;
use crate::cli::provider::{AWSConfigValidatedArgs, ProviderValidatedArgs};
use crate::cli::queue::QueueValidatedArgs;
use crate::cli::settlement::SettlementValidatedArgs;
use crate::cli::snos::SNOSParams;
use crate::cli::storage::StorageValidatedArgs;
use crate::cli::RunCmd;
use crate::data_storage::aws_s3::AWSS3;
use crate::data_storage::DataStorage;
use crate::database::mongodb::MongoDb;
use crate::database::Database;
use crate::helpers::{JobProcessingState, ProcessingLocks};
use crate::queue::sqs::SqsQueue;
use crate::queue::QueueProvider;
use crate::routes::ServerParams;

/// The app config. It can be accessed from anywhere inside the service
/// by calling `config` function.
pub struct Config {
    /// The orchestrator config
    orchestrator_params: OrchestratorParams,
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
    /// Locks
    processing_locks: ProcessingLocks,
}

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub max_block_to_process: Option<u64>,
    pub min_block_to_process: Option<u64>,
    pub max_concurrent_snos_jobs: Option<usize>,
    pub max_concurrent_proving_jobs: Option<usize>,
}

pub struct OrchestratorParams {
    pub madara_rpc_url: Url,
    pub snos_config: SNOSParams,
    pub service_config: ServiceParams,
    pub server_config: ServerParams,
    /// Layout to use for running SNOS
    pub snos_layout_name: LayoutName,
    /// Layout to use for proving
    pub prover_layout_name: LayoutName,
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
pub async fn get_aws_config(aws_config: &AWSConfigValidatedArgs) -> SdkConfig {
    let region = aws_config.aws_region.clone();
    let region_provider = RegionProviderChain::first_try(Region::new(region)).or_default_provider();
    let credentials =
        Credentials::from_keys(aws_config.aws_access_key_id.clone(), aws_config.aws_secret_access_key.clone(), None);
    aws_config::from_env().credentials_provider(credentials).region(region_provider).load().await
}

/// Initializes the app config
pub async fn init_config(run_cmd: &RunCmd) -> color_eyre::Result<Arc<Config>> {
    dotenv().ok();

    let provider_params = run_cmd.validate_provider_params().expect("Failed to validate provider params");
    let provider_config = build_provider_config(&provider_params).await;

    let (snos_layout_name, prover_layout_name) =
        run_cmd.validate_proving_layout_name().expect("Failed to validate prover layout name");

    let orchestrator_params = OrchestratorParams {
        madara_rpc_url: run_cmd.madara_rpc_url.clone(),
        snos_config: run_cmd.validate_snos_params().expect("Failed to validate SNOS params"),
        service_config: run_cmd.validate_service_params().expect("Failed to validate service params"),
        server_config: run_cmd.validate_server_params().expect("Failed to validate server params"),
        snos_layout_name,
        prover_layout_name,
    };

    let rpc_client = JsonRpcClient::new(HttpTransport::new(orchestrator_params.madara_rpc_url.clone()));

    // init database
    let database_params =
        run_cmd.validate_database_params().map_err(|e| eyre!("Failed to validate database params: {e}"))?;
    let database = build_database_client(&database_params).await;

    // init DA client
    let da_params = run_cmd.validate_da_params().map_err(|e| eyre!("Failed to validate DA params: {e}"))?;
    let da_client = build_da_client(&da_params).await;

    // init settlement
    let settlement_params =
        run_cmd.validate_settlement_params().map_err(|e| eyre!("Failed to validate settlement params: {e}"))?;
    let settlement_client = build_settlement_client(&settlement_params).await?;

    // init prover
    let prover_params = run_cmd.validate_prover_params().map_err(|e| eyre!("Failed to validate prover params: {e}"))?;
    let prover_client = build_prover_service(&prover_params);

    // init storage
    let data_storage_params =
        run_cmd.validate_storage_params().map_err(|e| eyre!("Failed to validate storage params: {e}"))?;
    let storage_client = build_storage_client(&data_storage_params, provider_config.clone()).await;

    // init alerts
    let alert_params = run_cmd.validate_alert_params().map_err(|e| eyre!("Failed to validate alert params: {e}"))?;
    let alerts_client = build_alert_client(&alert_params, provider_config.clone()).await;

    // init the queue
    // TODO: we use omniqueue for now which doesn't support loading AWS config
    // from `SdkConfig`. We can later move to using `aws_sdk_sqs`. This would require
    // us stop using the generic omniqueue abstractions for message ack/nack
    // init queue
    let queue_params = run_cmd.validate_queue_params().map_err(|e| eyre!("Failed to validate queue params: {e}"))?;
    let queue = build_queue_client(&queue_params, provider_config.clone()).await;

    let snos_processing_lock =
        JobProcessingState::new(orchestrator_params.service_config.max_concurrent_snos_jobs.unwrap_or(1));
    let proving_processing_lock =
        JobProcessingState::new(orchestrator_params.service_config.max_concurrent_proving_jobs.unwrap_or(1));
    let processing_locks = ProcessingLocks {
        snos_job_processing_lock: Arc::new(snos_processing_lock),
        proving_job_processing_lock: Arc::new(proving_processing_lock),
    };

    Ok(Arc::new(Config::new(
        orchestrator_params,
        Arc::new(rpc_client),
        da_client,
        prover_client,
        settlement_client,
        database,
        queue,
        storage_client,
        alerts_client,
        processing_locks,
    )))
}

impl Config {
    /// Create a new config
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        orchestrator_params: OrchestratorParams,
        starknet_client: Arc<JsonRpcClient<HttpTransport>>,
        da_client: Box<dyn DaClient>,
        prover_client: Box<dyn ProverClient>,
        settlement_client: Box<dyn SettlementClient>,
        database: Box<dyn Database>,
        queue: Box<dyn QueueProvider>,
        storage: Box<dyn DataStorage>,
        alerts: Box<dyn Alerts>,
        processing_locks: ProcessingLocks,
    ) -> Self {
        Self {
            orchestrator_params,
            starknet_client,
            da_client,
            prover_client,
            settlement_client,
            database,
            queue,
            storage,
            alerts,
            processing_locks,
        }
    }

    /// Returns the starknet rpc url
    pub fn starknet_rpc_url(&self) -> &Url {
        &self.orchestrator_params.madara_rpc_url
    }

    /// Returns the server config
    pub fn server_config(&self) -> &ServerParams {
        &self.orchestrator_params.server_config
    }

    /// Returns the snos rpc url
    pub fn snos_config(&self) -> &SNOSParams {
        &self.orchestrator_params.snos_config
    }

    /// Returns the service config
    pub fn service_config(&self) -> &ServiceParams {
        &self.orchestrator_params.service_config
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

    /// Returns the snos proof layout
    pub fn snos_layout_name(&self) -> &LayoutName {
        &self.orchestrator_params.snos_layout_name
    }

    /// Returns the snos proof layout
    pub fn prover_layout_name(&self) -> &LayoutName {
        &self.orchestrator_params.prover_layout_name
    }

    /// Returns the processing locks
    pub fn processing_locks(&self) -> &ProcessingLocks {
        &self.processing_locks
    }
}

/// Builds the provider config
pub async fn build_provider_config(provider_params: &ProviderValidatedArgs) -> Arc<ProviderConfig> {
    match provider_params {
        ProviderValidatedArgs::AWS(aws_params) => {
            Arc::new(ProviderConfig::AWS(Box::new(get_aws_config(aws_params).await)))
        }
    }
}

/// Builds the DA client based on the environment variable DA_LAYER
pub async fn build_da_client(da_params: &DaValidatedArgs) -> Box<dyn DaClient + Send + Sync> {
    match da_params {
        DaValidatedArgs::Ethereum(ethereum_da_params) => {
            Box::new(EthereumDaClient::new_with_args(ethereum_da_params).await)
        }
    }
}

/// Builds the prover service based on the environment variable PROVER_SERVICE
pub fn build_prover_service(prover_params: &ProverValidatedArgs) -> Box<dyn ProverClient> {
    match prover_params {
        ProverValidatedArgs::Sharp(sharp_params) => Box::new(SharpProverService::new_with_args(sharp_params)),
        ProverValidatedArgs::Atlantic(atlantic_params) => {
            Box::new(AtlanticProverService::new_with_args(atlantic_params))
        }
    }
}

/// Builds the settlement client depending on the env variable SETTLEMENT_LAYER
pub async fn build_settlement_client(
    settlement_params: &SettlementValidatedArgs,
) -> color_eyre::Result<Box<dyn SettlementClient + Send + Sync>> {
    match settlement_params {
        SettlementValidatedArgs::Ethereum(ethereum_settlement_params) => {
            #[cfg(not(feature = "testing"))]
            {
                Ok(Box::new(EthereumSettlementClient::new_with_args(ethereum_settlement_params)))
            }
            #[cfg(feature = "testing")]
            {
                Ok(Box::new(EthereumSettlementClient::with_test_params(
                    RootProvider::new_http(ethereum_settlement_params.ethereum_rpc_url.clone()),
                    ethereum_settlement_params.l1_core_contract_address,
                    ethereum_settlement_params.ethereum_rpc_url.clone(),
                    Some(ethereum_settlement_params.starknet_operator_address),
                )))
            }
        }
        SettlementValidatedArgs::Starknet(starknet_settlement_params) => {
            Ok(Box::new(StarknetSettlementClient::new_with_args(starknet_settlement_params).await))
        }
    }
}

pub async fn build_storage_client(
    data_storage_params: &StorageValidatedArgs,
    provider_config: Arc<ProviderConfig>,
) -> Box<dyn DataStorage + Send + Sync> {
    match data_storage_params {
        StorageValidatedArgs::AWSS3(aws_s3_params) => {
            let aws_config = provider_config.get_aws_client_or_panic();
            Box::new(AWSS3::new_with_args(aws_s3_params, aws_config).await)
        }
    }
}

pub async fn build_alert_client(
    alert_params: &AlertValidatedArgs,
    provider_config: Arc<ProviderConfig>,
) -> Box<dyn Alerts + Send + Sync> {
    match alert_params {
        AlertValidatedArgs::AWSSNS(aws_sns_params) => {
            let aws_config = provider_config.get_aws_client_or_panic();
            Box::new(AWSSNS::new_with_args(aws_sns_params, aws_config).await)
        }
    }
}

pub async fn build_queue_client(
    queue_params: &QueueValidatedArgs,
    provider_config: Arc<ProviderConfig>,
) -> Box<dyn QueueProvider + Send + Sync> {
    match queue_params {
        QueueValidatedArgs::AWSSQS(aws_sqs_params) => {
            let aws_config = provider_config.get_aws_client_or_panic();
            Box::new(SqsQueue::new_with_args(aws_sqs_params.clone(), aws_config))
        }
    }
}

pub async fn build_database_client(database_params: &DatabaseValidatedArgs) -> Box<dyn Database + Send + Sync> {
    match database_params {
        DatabaseValidatedArgs::MongoDB(mongodb_params) => Box::new(MongoDb::new_with_args(mongodb_params).await),
    }
}
