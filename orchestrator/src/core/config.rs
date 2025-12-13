#[cfg(feature = "testing")]
use alloy::providers::ProviderBuilder;
use cairo_vm::Felt252;

use crate::utils::rest_client::RestClient;
use anyhow::Context;
use cairo_vm::types::layout_name::LayoutName;
use orchestrator_atlantic_service::AtlanticProverService;
use orchestrator_da_client_interface::DaClient;
use orchestrator_ethereum_da_client::EthereumDaClient;
use orchestrator_ethereum_settlement_client::EthereumSettlementClient;
use orchestrator_prover_client_interface::ProverClient;
use orchestrator_settlement_client_interface::SettlementClient;
use orchestrator_sharp_service::SharpProverService;
use orchestrator_starknet_da_client::StarknetDaClient;
use orchestrator_starknet_settlement_client::StarknetSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info};
use url::Url;

use crate::core::client::lock::mongodb::MongoLockClient;
use crate::core::client::lock::LockClient;
use crate::core::error::OrchestratorCoreResult;
use crate::types::params::batching::BatchingParams;
use crate::types::params::database::DatabaseArgs;
use crate::types::Layer;
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
    OrchestratorError, OrchestratorResult,
};

use crate::types::batch::AggregatorBatchWeights;
use blockifier::bouncer::BouncerWeights;

/// Starknet versions supported by the service
macro_rules! versions {
    ($(($variant:ident, $version:expr)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum StarknetVersion {
            $($variant),*
        }

        impl StarknetVersion {
            pub fn to_string(&self) -> &'static str {
                match self {
                    $(Self::$variant => $version),*
                }
            }

            pub fn supported() -> &'static [StarknetVersion] {
                &[$(Self::$variant),*]
            }

            pub fn is_supported(&self) -> bool {
                Self::supported().contains(self)
            }
        }

        impl FromStr for StarknetVersion {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($version => Ok(Self::$variant),)*
                    _ => Err(format!("Unknown version: {}", s)),
                }
            }
        }

        /// Making 0.13.3 as the default version for now
        impl Default for StarknetVersion {
            fn default() -> Self {
                Self::V0_13_3
            }
        }

        impl std::fmt::Display for StarknetVersion {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.to_string())
            }
        }

        impl serde::Serialize for StarknetVersion {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.to_string())
            }
        }

        impl<'de> serde::Deserialize<'de> for StarknetVersion {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                StarknetVersion::from_str(&s).map_err(serde::de::Error::custom)
            }
        }
    }
}

// Add more versions here whenever necessary. Follow the following rules:
// 1. Make sure that the versions are ordered (for e.g., 0.15.0 must come after 0.14.0)
// 2. In the env, use the dot notation, i.e., if you want to run it for "0.13.2", pass this in env
versions!(
    (V0_13_2, "0.13.2"),
    (V0_13_3, "0.13.3"),
    (V0_13_4, "0.13.4"),
    (V0_13_5, "0.13.5"),
    (V0_14_0, "0.14.0"),
    (V0_14_1, "0.14.1")
);

#[derive(Debug, Clone)]
pub struct ConfigParam {
    pub madara_rpc_url: Url,
    pub madara_feeder_gateway_url: Url,
    pub madara_version: StarknetVersion,
    pub snos_config: SNOSParams,
    pub batching_config: BatchingParams,
    pub service_config: ServiceParams,
    pub server_config: ServerParams,
    /// Layout to use for running SNOS
    pub snos_layout_name: LayoutName,
    /// Layout to use for proving
    pub prover_layout_name: LayoutName,
    /// Specifies if we should store job artifacts which are not used in the code i.e., not
    /// necessary for the application, but can be used for auditing purposes.
    ///
    /// Currently, being used to check if we should store
    /// * Aggregator Proof
    pub store_audit_artifacts: bool,
    pub bouncer_weights_limit: BouncerWeights,
    pub aggregator_batch_weights_limit: AggregatorBatchWeights,
}

/// The app config. It can be accessed from anywhere inside the service
/// by calling the ` config ` function. 33
pub struct Config {
    layer: Layer,
    /// The orchestrator config
    pub params: ConfigParam,
    /// The Madara client to get data from the node
    madara_rpc_client: Arc<JsonRpcClient<HttpTransport>>,
    /// The Madara feeder gateway client for fetching builtins
    madara_feeder_gateway_client: Arc<RestClient>,
    /// The DA client to interact with the DA layer
    da_client: Box<dyn DaClient>,
    /// The service that produces proof and registers it onchain
    prover_client: Box<dyn ProverClient>,
    /// Settlement client
    settlement_client: Box<dyn SettlementClient>,
    /// The database client
    database: Box<dyn DatabaseClient>,
    /// Lock client
    lock: Box<dyn LockClient>,
    /// Queue client
    queue: Box<dyn QueueClient>,
    /// Storage client
    storage: Box<dyn StorageClient>,
    /// Alerts client
    alerts: Box<dyn AlertClient>,
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn new(
        layer: Layer,
        params: ConfigParam,
        madara_rpc_client: Arc<JsonRpcClient<HttpTransport>>,
        madara_feeder_gateway_client: Arc<RestClient>,
        database: Box<dyn DatabaseClient>,
        storage: Box<dyn StorageClient>,
        lock: Box<dyn LockClient>,
        alerts: Box<dyn AlertClient>,
        queue: Box<dyn QueueClient>,
        prover_client: Box<dyn ProverClient>,
        da_client: Box<dyn DaClient>,
        settlement_client: Box<dyn SettlementClient>,
    ) -> Self {
        Self {
            layer,
            params,
            madara_rpc_client,
            madara_feeder_gateway_client,
            database,
            lock,
            storage,
            alerts,
            queue,
            prover_client,
            da_client,
            settlement_client,
        }
    }

    /// new - create config from the run command
    pub async fn from_run_cmd(run_cmd: &RunCmd) -> OrchestratorResult<Self> {
        let cloud_provider =
            CloudProvider::try_from(run_cmd.clone()).context("Failed to create cloud provider from run command")?;
        let provider_config = Arc::new(cloud_provider);

        let db: DatabaseArgs =
            DatabaseArgs::try_from(run_cmd.clone()).context("Failed to create database args from run command")?;
        let storage_args: StorageArgs =
            StorageArgs::try_from(run_cmd.clone()).context("Failed to create storage args from run command")?;
        let alert_args: AlertArgs =
            AlertArgs::try_from(run_cmd.clone()).context("Failed to create alert args from run command")?;
        let queue_args: QueueArgs =
            QueueArgs::try_from(run_cmd.clone()).context("Failed to create queue args from run command")?;

        let prover_config =
            ProverConfig::try_from(run_cmd.clone()).context("Failed to create prover config from run command")?;
        let da_config = DAConfig::try_from(run_cmd.clone()).context("Failed to create DA config from run command")?;
        let settlement_config = SettlementConfig::try_from(run_cmd.clone())
            .context("Failed to create settlement config from run command")?;

        let bouncer_weights_limit = Self::load_bouncer_weights_limit(&run_cmd.bouncer_weights_limit_file)?;

        let layer = run_cmd.layer.clone();

        let params = ConfigParam {
            madara_rpc_url: run_cmd.madara_rpc_url.clone(),
            madara_feeder_gateway_url: run_cmd
                .madara_feeder_gateway_url
                .clone()
                .unwrap_or_else(|| run_cmd.madara_rpc_url.clone()),
            madara_version: run_cmd.madara_version,
            snos_config: SNOSParams::from(run_cmd.snos_args.clone()),
            batching_config: BatchingParams::from(run_cmd.batching_args.clone()),
            service_config: ServiceParams::from(run_cmd.service_args.clone()),
            server_config: ServerParams::from(run_cmd.server_args.clone()),
            snos_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.snos_layout_name.clone().as_str())
                .context("Failed to get SNOS layout name")?,
            prover_layout_name: Self::get_layout_name(run_cmd.proving_layout_args.prover_layout_name.clone().as_str())
                .context("Failed to get prover layout name")?,
            store_audit_artifacts: run_cmd.store_audit_artifacts,
            aggregator_batch_weights_limit: AggregatorBatchWeights::from(&bouncer_weights_limit),
            bouncer_weights_limit,
        };
        let rpc_client = JsonRpcClient::new(HttpTransport::new(params.madara_rpc_url.clone()));
        let feeder_gateway_client = RestClient::new(params.madara_feeder_gateway_url.clone());

        let database = Self::build_database_client(&db).await?;
        let lock = Self::build_lock_client(&db).await?;
        let storage = Self::build_storage_client(&storage_args, provider_config.clone()).await?;
        let alerts = Self::build_alert_client(&alert_args, provider_config.clone()).await?;
        let queue = Self::build_queue_client(&queue_args, provider_config.clone()).await?;

        // Start mock Atlantic server if flag is enabled
        if run_cmd.mock_atlantic_server {
            Self::start_mock_atlantic_server(&prover_config, run_cmd.mock_atlantic_server).await;
        }

        // External Clients Initialization
        let prover_client = Self::build_prover_service(
            &prover_config,
            &params,
            Some(
                rpc_client
                    .chain_id()
                    .await
                    .map_err(|e| OrchestratorError::ConfigError(format!("Failed to get Chain ID from RPC: {}", e)))?
                    .to_fixed_hex_string(),
            ),
            Some(Felt252::from_str(params.snos_config.strk_fee_token_address.clone().as_str())?),
        );
        let da_client: Box<dyn DaClient + Send + Sync + 'static> = Self::build_da_client(&da_config).await;
        let settlement_client = Self::build_settlement_client(&settlement_config).await?;

        Ok(Self {
            layer,
            params,
            madara_rpc_client: Arc::new(rpc_client),
            madara_feeder_gateway_client: Arc::new(feeder_gateway_client),
            database,
            lock,
            storage,
            alerts,
            queue,
            prover_client,
            da_client,
            settlement_client,
        })
    }

    /// Returns the layer of the config
    pub fn layer(&self) -> &Layer {
        &self.layer
    }

    pub(crate) async fn build_database_client(
        db_args: &DatabaseArgs,
    ) -> OrchestratorCoreResult<Box<dyn DatabaseClient + Send + Sync>> {
        Ok(Box::new(MongoDbClient::new(db_args).await?))
    }

    pub(crate) async fn build_lock_client(
        args: &DatabaseArgs,
    ) -> OrchestratorCoreResult<Box<dyn LockClient + Send + Sync>> {
        Ok(Box::new(MongoLockClient::new(args).await?))
    }

    pub(crate) async fn build_storage_client(
        storage_config: &StorageArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn StorageClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(AWSS3::new(aws_config, storage_config)))
    }

    pub(crate) async fn build_alert_client(
        alert_config: &AlertArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn AlertClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SNS::new(aws_config, alert_config)))
    }

    pub(crate) async fn build_queue_client(
        queue_config: &QueueArgs,
        provider_config: Arc<CloudProvider>,
    ) -> OrchestratorCoreResult<Box<dyn QueueClient + Send + Sync>> {
        let aws_config = provider_config.get_aws_client_or_panic();
        Ok(Box::new(SQS::new(aws_config, queue_config)))
    }

    /// build_prover_service - Build the proving service based on the config
    ///
    /// # Arguments
    /// * `prover_params` - The proving service parameters
    /// * `params` - The config parameters
    /// * `chain_id_hex` - The chain ID in hex format
    /// * `fee_token_address` - The fee token address
    /// # Returns
    /// * `Box<dyn ProverClient>` - The proving service
    pub(crate) fn build_prover_service(
        prover_params: &ProverConfig,
        params: &ConfigParam,
        chain_id_hex: Option<String>,
        fee_token_address: Option<Felt252>,
    ) -> Box<dyn ProverClient + Send + Sync> {
        match prover_params {
            ProverConfig::Sharp(sharp_params) => {
                Box::new(SharpProverService::new_with_args(sharp_params, &params.prover_layout_name))
            }
            ProverConfig::Atlantic(atlantic_params) => Box::new(AtlanticProverService::new_with_args(
                atlantic_params,
                &params.prover_layout_name,
                chain_id_hex,
                fee_token_address,
            )),
        }
    }

    /// start_mock_atlantic_server - Start the mock Atlantic server
    ///
    /// # Arguments
    /// * `prover_params` - The proving service parameters
    /// * `allow_mock_hash_server` - Whether to allow the mock Atlantic server
    /// # Returns
    /// * `Box<dyn ProverClient>` - The proving service
    ///
    /// # Notes
    /// This function is used to start the mock Atlantic server if the flag is enabled and we're in testnet.
    /// It starts the mock server in a background task and gives it time to start.
    async fn start_mock_atlantic_server(prover_params: &ProverConfig, allow_mock_hash_server: bool) {
        match prover_params {
            ProverConfig::Atlantic(atlantic_params) => {
                // Start mock Atlantic server if flag is enabled and we're in testnet
                if allow_mock_hash_server && atlantic_params.atlantic_network == "TESTNET" {
                    info!("Mock Atlantic server flag is enabled, starting mock server...");

                    // Start the mock server in a background task
                    tokio::spawn(async move {
                        info!("Starting mock Atlantic server on port 4001");
                        if let Err(e) = utils_mock_atlantic_server::start_mock_atlantic_server().await {
                            error!("Failed to start mock Atlantic server: {}", e);
                        }
                    });

                    // Give the mock server time to start
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    info!("Mock Atlantic server started successfully");
                }
            }
            ProverConfig::Sharp(_) => {
                tracing::warn!("Mock Atlantic server flag is enabled, but prover is not Atlantic");
            }
        }
    }

    pub(crate) async fn build_da_client(da_params: &DAConfig) -> Box<dyn DaClient + Send + Sync> {
        match da_params {
            DAConfig::Ethereum(ethereum_da_params) => {
                Box::new(EthereumDaClient::new_with_args(ethereum_da_params).await)
            }
            DAConfig::Starknet(starknet_da_params) => {
                Box::new(StarknetDaClient::new_with_args(starknet_da_params).await)
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
                        ProviderBuilder::new().connect_http(ethereum_settlement_params.ethereum_rpc_url.clone()),
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
    pub fn madara_rpc_client(&self) -> &Arc<JsonRpcClient<HttpTransport>> {
        &self.madara_rpc_client
    }

    /// Returns the Madara feeder gateway client
    pub fn madara_feeder_gateway_client(&self) -> &Arc<RestClient> {
        &self.madara_feeder_gateway_client
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

    /// Returns the Lock Client
    pub fn lock(&self) -> &dyn LockClient {
        self.lock.as_ref()
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

    /// Returns the bouncer weights limit
    pub fn bouncer_weights_limit(&self) -> &BouncerWeights {
        &self.params.bouncer_weights_limit
    }

    /// Load bouncer weights limit from file or use defaults
    fn load_bouncer_weights_limit(file_path: &Option<std::path::PathBuf>) -> OrchestratorCoreResult<BouncerWeights> {
        match file_path {
            Some(path) => {
                tracing::info!(file_path = %path.display(), "Loading bouncer weights limit from file");

                match std::fs::read_to_string(path) {
                    Ok(content) => match serde_json::from_str::<BouncerWeights>(&content) {
                        Ok(weights) => {
                            tracing::info!("Successfully loaded bouncer weights limit from file");
                            Ok(weights)
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                file_path = %path.display(),
                                "Failed to parse bouncer weights limit file, using defaults"
                            );
                            Ok(Self::default_bouncer_weights_limit())
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            file_path = %path.display(),
                            "Failed to read bouncer weights limit file, using defaults"
                        );
                        Ok(Self::default_bouncer_weights_limit())
                    }
                }
            }
            None => {
                tracing::warn!("No bouncer weights limit file provided, using default values");
                Ok(Self::default_bouncer_weights_limit())
            }
        }
    }

    /// Default bouncer weights limit values
    fn default_bouncer_weights_limit() -> BouncerWeights {
        use starknet_api::execution_resources::GasAmount;

        BouncerWeights {
            l1_gas: 1_000_000,                 // 1M L1 gas
            message_segment_length: 100_000,   // 100K message segment length
            n_events: 5_000,                   // 5K events
            state_diff_size: 100_000,          // 100K state diff size
            sierra_gas: GasAmount(10_000_000), // 10M sierra gas
            n_txs: 1_000,                      // 1K transactions
            proving_gas: GasAmount(5_000_000), // 5M proving gas
        }
    }
}
