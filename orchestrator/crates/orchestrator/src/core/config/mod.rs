use alloy::providers::RootProvider;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_atlantic_service::AtlanticProverService;
use orchestrator_da_client_interface::DaClient;
use orchestrator_ethereum_da_client::EthereumDaClient;
use orchestrator_ethereum_settlement_client::EthereumSettlementClient;
use orchestrator_prover_client_interface::ProverClient;
use orchestrator_settlement_client_interface::SettlementClient;
use orchestrator_sharp_service::SharpProverService;
use orchestrator_starknet_settlement_client::StarknetSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::sync::Arc;
use url::Url;

use crate::core::error::OrchestratorCoreResult;
use crate::{
    cli::RunCmd,
    cli::ServiceParams,
    core::client::{
        queue::QueueClient, storage::sss::AWSS3, storage::StorageClient, AlertClient, DatabaseClient, MongoDbClient,
        SNS, SQS,
    },
    core::cloud::CloudProvider,
    types::params::cloud_provider::AWSCredentials,
    types::params::da::DAConfig,
    types::params::database::MongoConfig,
    types::params::prover::ProverConfig,
    types::params::service::ServerParams,
    types::params::settlement::SettlementConfig,
    types::params::snos::SNOSParams,
    types::params::{AlertArgs, QueueArgs, StorageArgs},
    utils::helpers::{JobProcessingState, ProcessingLocks},
    OrchestratorError, OrchestratorResult,
};

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

/// The app config. It can be accessed from anywhere inside the service
/// by calling `config` function. 33
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
    database: Box<dyn DatabaseClient>,
    /// Queue client
    queue: Box<dyn QueueClient>,
    /// Storage client
    storage: Box<dyn StorageClient>,
    /// Alerts client
    alerts: Box<dyn AlertClient>,
    /// Locks
    processing_locks: ProcessingLocks,
}

impl Config {
    /// Setup the orchestrator
    pub async fn setup(run_cmd: &RunCmd) -> OrchestratorResult<Self> {
        let aws_cred = AWSCredentials::from(run_cmd.aws_config_args.clone());
        let cloud_provider = aws_cred.get_aws_config().await;
        let provider_config = Arc::new(CloudProvider::AWS(Box::new(cloud_provider)));

        let db: MongoConfig = run_cmd.mongodb_args.clone().into();
        let storage_args: StorageArgs = StorageArgs::try_from(run_cmd.clone())?;
        let alert_args: AlertArgs = run_cmd.aws_sns_args.clone().try_into()?;
        let queue_args: QueueArgs = QueueArgs::try_from(run_cmd.clone())?;

        let prover_config = ProverConfig::try_from(run_cmd.clone())?;
        let da_config = DAConfig::try_from(run_cmd.ethereum_da_args.clone())?;
        let settlement_config = SettlementConfig::try_from(run_cmd.clone())?;

        let orchestrator_params = OrchestratorParams {
            madara_rpc_url: run_cmd.madara_rpc_url.clone(),
            snos_config: run_cmd.snos_args.clone().into(),
            service_config: run_cmd.service_args.clone().into(),
            server_config: ServerParams::from(run_cmd.server_args.clone()),
            snos_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.prover_layout_name.clone().as_str())?,
            prover_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.snos_layout_name.clone().as_str())?,
        };
        let rpc_client = JsonRpcClient::new(HttpTransport::new(orchestrator_params.madara_rpc_url.clone()));
        let snos_processing_lock =
            JobProcessingState::new(orchestrator_params.service_config.max_concurrent_snos_jobs.unwrap_or(1));
        let processing_locks = ProcessingLocks { snos_job_processing_lock: Arc::new(snos_processing_lock) };

        let database = Self::build_database_client(&db).await?;
        let storage = Self::build_storage_client(&storage_args, provider_config.clone()).await?;
        let alerts = Self::build_alert_client(&alert_args, provider_config.clone()).await?;
        let queue = Self::build_queue_client(&queue_args, provider_config.clone()).await?;

        // External Clients Initialization
        let prover_client = Self::build_prover_service(&prover_config);
        let da_client = Self::build_da_client(&da_config).await;
        let settlement_client = Self::build_settlement_client(&settlement_config).await?;

        Ok(Self {
            orchestrator_params,
            starknet_client: Arc::new(rpc_client),
            database,
            storage,
            alerts,
            queue,
            prover_client,
            da_client,
            processing_locks,
            settlement_client,
        })
    }

    async fn build_database_client(
        db_config: &MongoConfig,
    ) -> OrchestratorResult<Box<dyn DatabaseClient + Send + Sync>> {
        Ok(Box::new(MongoDbClient::setup(db_config).await?))
    }

    async fn build_storage_client(
        storage_config: &StorageArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn StorageClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(AWSS3::create(storage_config, aws_config).await?))
    }

    async fn build_alert_client(
        alert_config: &AlertArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn AlertClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SNS::create(alert_config, aws_config)))
    }

    async fn build_queue_client(
        queue_config: &QueueArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn QueueClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SQS::create(queue_config, aws_config)?))
    }

    fn build_prover_service(prover_params: &ProverConfig) -> Box<dyn ProverClient> {
        match prover_params {
            ProverConfig::Sharp(sharp_params) => Box::new(SharpProverService::new_with_args(sharp_params)),
            ProverConfig::Atlantic(atlantic_params) => Box::new(AtlanticProverService::new_with_args(atlantic_params)),
        }
    }

    async fn build_da_client(da_params: &DAConfig) -> Box<dyn DaClient + Send + Sync> {
        match da_params {
            DAConfig::Ethereum(ethereum_da_params) => {
                Box::new(EthereumDaClient::new_with_args(ethereum_da_params).await)
            }
        }
    }

    async fn build_settlement_client(
        settlement_params: &SettlementConfig,
    ) -> OrchestratorResult<Box<dyn SettlementClient + Send + Sync>> {
        match settlement_params {
            SettlementConfig::Ethereum(ethereum_settlement_params) => {
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
            SettlementConfig::Starknet(starknet_settlement_params) => {
                Ok(Box::new(StarknetSettlementClient::new_with_args(starknet_settlement_params).await))
            }
        }
    }

    /// get_layout_name - Returns the layout name based on the input string
    fn get_layout_name(layout_name: &str) -> OrchestratorResult<LayoutName> {
        Ok(match layout_name {
            "plain" => LayoutName::plain,
            "small" => LayoutName::small,
            "dex" => LayoutName::dex,
            "recursive" => LayoutName::recursive,
            "starknet" => LayoutName::starknet,
            "starknet_with_keccak" => LayoutName::starknet_with_keccak,
            "recursive_large_output" => LayoutName::recursive_large_output,
            "recursive_with_poseidon" => LayoutName::recursive_with_poseidon,
            "all_solidity" => LayoutName::all_solidity,
            "all_cairo" => LayoutName::all_cairo,
            "dynamic" => LayoutName::dynamic,
            _ => return Err(OrchestratorError::InvalidLayoutError(layout_name.to_string())),
        })
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
    pub fn database(&self) -> &dyn DatabaseClient {
        self.database.as_ref()
    }

    /// Returns the queue provider
    pub fn queue(&self) -> &dyn QueueClient {
        self.queue.as_ref()
    }

    /// Returns the storage provider
    pub fn storage(&self) -> &dyn StorageClient {
        self.storage.as_ref()
    }

    /// Returns the alerts client
    pub fn alerts(&self) -> &dyn AlertClient {
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
