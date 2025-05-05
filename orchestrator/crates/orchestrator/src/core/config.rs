#[cfg(feature = "testing")]
use alloy::providers::RootProvider;

use cairo_vm::types::layout_name::LayoutName;
use orchestrator_atlantic_service::AtlanticProverService;
use orchestrator_da_client_interface::DaClient;
use orchestrator_ethereum_da_client::EthereumDaClient;
use orchestrator_ethereum_settlement_client::EthereumSettlementClient;
use orchestrator_settlement_client_interface::SettlementClient;

use orchestrator_prover_client_interface::ProverClient;
use orchestrator_sharp_service::SharpProverService;
use orchestrator_starknet_settlement_client::StarknetSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::sync::Arc;
use url::Url;

use crate::core::error::OrchestratorCoreResult;
use crate::types::params::database::DatabaseArgs;
use crate::{
    cli::RunCmd,
    core::client::{
        queue::QueueClient, storage::s3::AWSS3, storage::StorageClient, AlertClient, DatabaseClient, MongoDbClient,
        SNS, SQS,
    },
    core::cloud::CloudProvider,
    types::params::da::DAConfig,
    types::params::prover::ProverConfig,
    types::params::service::{ServerParams, ServiceParams},
    types::params::settlement::SettlementConfig,
    types::params::snos::SNOSParams,
    types::params::{AlertArgs, QueueArgs, StorageArgs},
    utils::helpers::{JobProcessingState, ProcessingLocks},
    OrchestratorError, OrchestratorResult,
};

#[derive(Debug, Clone)]
pub struct ConfigParam {
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
    params: ConfigParam,
    /// The Madara client to get data from the node
    madara_client: Arc<JsonRpcClient<HttpTransport>>,
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
    #[allow(clippy::too_many_arguments, dead_code)]
    pub(crate) fn new(
        params: ConfigParam,
        madara_client: Arc<JsonRpcClient<HttpTransport>>,
        database: Box<dyn DatabaseClient>,
        storage: Box<dyn StorageClient>,
        alerts: Box<dyn AlertClient>,
        queue: Box<dyn QueueClient>,
        prover_client: Box<dyn ProverClient>,
        da_client: Box<dyn DaClient>,
        processing_locks: ProcessingLocks,
        settlement_client: Box<dyn SettlementClient>,
    ) -> Self {
        Self {
            params,
            madara_client,
            database,
            storage,
            alerts,
            queue,
            prover_client,
            da_client,
            processing_locks,
            settlement_client,
        }
    }

    /// new - create config from the run command
    pub async fn from_run_cmd(run_cmd: &RunCmd) -> OrchestratorResult<Self> {
        let cloud_provider = CloudProvider::try_from(run_cmd.clone())?;
        let provider_config = Arc::new(cloud_provider);

        let db: DatabaseArgs = DatabaseArgs::try_from(run_cmd.clone())?;
        let storage_args: StorageArgs = StorageArgs::try_from(run_cmd.clone())?;
        let alert_args: AlertArgs = AlertArgs::try_from(run_cmd.clone())?;
        let queue_args: QueueArgs = QueueArgs::try_from(run_cmd.clone())?;

        let prover_config = ProverConfig::try_from(run_cmd.clone())?;
        let da_config = DAConfig::try_from(run_cmd.clone())?;
        let settlement_config = SettlementConfig::try_from(run_cmd.clone())?;

        let params = ConfigParam {
            madara_rpc_url: run_cmd.madara_rpc_url.clone(),
            snos_config: SNOSParams::from(run_cmd.snos_args.clone()),
            service_config: ServiceParams::from(run_cmd.service_args.clone()),
            server_config: ServerParams::from(run_cmd.server_args.clone()),
            snos_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.prover_layout_name.clone().as_str())?,
            prover_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.snos_layout_name.clone().as_str())?,
        };
        let rpc_client = JsonRpcClient::new(HttpTransport::new(params.madara_rpc_url.clone()));

        let mut processing_locks = ProcessingLocks::default();

        if let Some(max_concurrent_snos_jobs) = params.service_config.max_concurrent_snos_jobs {
            processing_locks.snos_job_processing_lock =
                Some(Arc::new(JobProcessingState::new(max_concurrent_snos_jobs)));
        }

        if let Some(max_concurrent_proving_jobs) = params.service_config.max_concurrent_proving_jobs {
            processing_locks.proving_job_processing_lock =
                Some(Arc::new(JobProcessingState::new(max_concurrent_proving_jobs)));
        }

        let database = Self::build_database_client(&db).await?;
        let storage = Self::build_storage_client(&storage_args, provider_config.clone()).await?;
        let alerts = Self::build_alert_client(&alert_args, provider_config.clone()).await?;
        let queue = Self::build_queue_client(&queue_args, provider_config.clone()).await?;

        // External Clients Initialization
        let prover_client = Self::build_prover_service(&prover_config, &params);
        let da_client = Self::build_da_client(&da_config).await;
        let settlement_client = Self::build_settlement_client(&settlement_config).await?;

        Ok(Self {
            params,
            madara_client: Arc::new(rpc_client),
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

    pub(crate) async fn build_database_client(
        db_args: &DatabaseArgs,
    ) -> OrchestratorCoreResult<Box<dyn DatabaseClient + Send + Sync>> {
        Ok(Box::new(MongoDbClient::new(db_args).await?))
    }

    pub(crate) async fn build_storage_client(
        storage_config: &StorageArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn StorageClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(AWSS3::new(aws_config, Some(storage_config))))
    }

    pub(crate) async fn build_alert_client(
        alert_config: &AlertArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn AlertClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SNS::new(aws_config, Some(alert_config))))
    }

    pub(crate) async fn build_queue_client(
        queue_config: &QueueArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn QueueClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SQS::new(aws_config, Some(queue_config))))
    }

    /// build_prover_service - Build the proving service based on the config
    ///
    /// # Arguments
    /// * `prover_params` - The proving service parameters
    /// * `params` - The config parameters
    /// # Returns
    /// * `Box<dyn ProverClient>` - The proving service
    pub(crate) fn build_prover_service(
        prover_params: &ProverConfig,
        params: &ConfigParam,
    ) -> Box<dyn ProverClient + Send + Sync> {
        match prover_params {
            ProverConfig::Sharp(sharp_params) => {
                Box::new(SharpProverService::new_with_args(sharp_params, &params.prover_layout_name))
            }
            ProverConfig::Atlantic(atlantic_params) => {
                Box::new(AtlanticProverService::new_with_args(atlantic_params, &params.prover_layout_name))
            }
        }
    }

    pub(crate) async fn build_da_client(da_params: &DAConfig) -> Box<dyn DaClient + Send + Sync> {
        match da_params {
            DAConfig::Ethereum(ethereum_da_params) => {
                Box::new(EthereumDaClient::new_with_args(ethereum_da_params).await)
            }
        }
    }

    pub(crate) async fn build_settlement_client(
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

    /// Returns the Madara client
    pub fn madara_client(&self) -> &Arc<JsonRpcClient<HttpTransport>> {
        &self.madara_client
    }

    /// Returns the server config
    pub fn server_config(&self) -> &ServerParams {
        &self.params.server_config
    }

    /// Returns the snos rpc url
    pub fn snos_config(&self) -> &SNOSParams {
        &self.params.snos_config
    }

    /// Returns the service config
    pub fn service_config(&self) -> &ServiceParams {
        &self.params.service_config
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

    /// Returns the alert client
    pub fn alerts(&self) -> &dyn AlertClient {
        self.alerts.as_ref()
    }

    /// Returns the snos proof layout
    pub fn snos_layout_name(&self) -> &LayoutName {
        &self.params.snos_layout_name
    }

    /// Returns the snos proof layout
    pub fn prover_layout_name(&self) -> &LayoutName {
        &self.params.prover_layout_name
    }

    /// Returns the processing locks
    pub fn processing_locks(&self) -> &ProcessingLocks {
        &self.processing_locks
    }
}
