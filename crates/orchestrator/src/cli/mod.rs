use alert::AlertValidatedArgs;
use cairo_vm::types::layout_name::LayoutName;
use clap::{ArgGroup, Parser, Subcommand};
use cron::event_bridge::AWSEventBridgeCliArgs;
use cron::CronValidatedArgs;
use da::DaValidatedArgs;
use database::DatabaseValidatedArgs;
use prover::ProverValidatedArgs;
use provider::aws::AWSConfigCliArgs;
use provider::ProviderValidatedArgs;
use queue::QueueValidatedArgs;
use snos::SNOSParams;
use storage::StorageValidatedArgs;
use url::Url;

use crate::config::ServiceParams;
use crate::routes::ServerParams;
use crate::telemetry::InstrumentationParams;

pub mod alert;
pub mod cron;
pub mod da;
pub mod database;
pub mod instrumentation;
pub mod prover;
pub mod prover_layout;
pub mod provider;
pub mod queue;
pub mod server;
pub mod service;
pub mod settlement;
pub mod snos;
pub mod storage;
#[derive(Parser, Debug)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the orchestrator
    Run {
        #[command(flatten)]
        run_command: Box<RunCmd>,
    },
    /// Setup the orchestrator
    Setup {
        #[command(flatten)]
        setup_command: Box<SetupCmd>,
    },
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[clap(
    group(
        ArgGroup::new("provider")
            .args(&["aws"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("settlement_layer")
            .args(&["settle_on_ethereum", "settle_on_starknet"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("storage")
            .args(&["aws_s3"])
            .required(true)
            .multiple(false)
            .requires("provider")
    ),
    group(
      ArgGroup::new("queue")
          .args(&["aws_sqs"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
      ArgGroup::new("alert")
          .args(&["aws_sns"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
        ArgGroup::new("prover")
            .args(&["sharp", "atlantic"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("da_layer")
            .args(&["da_on_ethereum"])
            .required(true)
            .multiple(false)
    ),
)]
pub struct RunCmd {
    // Provider Config
    #[clap(flatten)]
    pub aws_config_args: AWSConfigCliArgs,

    // Settlement Layer
    #[clap(flatten)]
    ethereum_args: settlement::ethereum::EthereumSettlementCliArgs,

    #[clap(flatten)]
    starknet_args: settlement::starknet::StarknetSettlementCliArgs,

    // Storage
    #[clap(flatten)]
    pub aws_s3_args: storage::aws_s3::AWSS3CliArgs,

    // Queue
    #[clap(flatten)]
    pub aws_sqs_args: queue::aws_sqs::AWSSQSCliArgs,

    // Server
    #[clap(flatten)]
    pub server_args: server::ServerCliArgs,

    // Alert
    #[clap(flatten)]
    pub aws_sns_args: alert::aws_sns::AWSSNSCliArgs,

    // Database
    #[clap(flatten)]
    pub mongodb_args: database::mongodb::MongoDBCliArgs,

    // Data Availability Layer
    #[clap(flatten)]
    pub ethereum_da_args: da::ethereum::EthereumDaCliArgs,

    // Prover
    #[clap(flatten)]
    pub sharp_args: prover::sharp::SharpCliArgs,

    #[clap(flatten)]
    pub atlantic_args: prover::atlantic::AtlanticCliArgs,

    #[clap(flatten)]
    pub proving_layout_args: prover_layout::ProverLayoutCliArgs,

    // SNOS
    #[clap(flatten)]
    pub snos_args: snos::SNOSCliArgs,

    #[arg(env = "MADARA_ORCHESTRATOR_MADARA_RPC_URL", long, required = true)]
    pub madara_rpc_url: Url,

    // Service
    #[clap(flatten)]
    pub service_args: service::ServiceCliArgs,
    #[clap(flatten)]
    pub instrumentation_args: instrumentation::InstrumentationCliArgs,
}

impl RunCmd {
    pub fn validate_provider_params(&self) -> Result<ProviderValidatedArgs, String> {
        validate_params::validate_provider_params(&self.aws_config_args)
    }

    pub fn validate_alert_params(&self) -> Result<AlertValidatedArgs, String> {
        validate_params::validate_alert_params(&self.aws_sns_args, &self.aws_config_args)
    }

    pub fn validate_queue_params(&self) -> Result<QueueValidatedArgs, String> {
        validate_params::validate_queue_params(&self.aws_sqs_args, &self.aws_config_args)
    }

    pub fn validate_storage_params(&self) -> Result<StorageValidatedArgs, String> {
        validate_params::validate_storage_params(&self.aws_s3_args, &self.aws_config_args)
    }

    pub fn validate_database_params(&self) -> Result<DatabaseValidatedArgs, String> {
        validate_params::validate_database_params(&self.mongodb_args)
    }

    pub fn validate_da_params(&self) -> Result<DaValidatedArgs, String> {
        validate_params::validate_da_params(&self.ethereum_da_args)
    }

    pub fn validate_settlement_params(&self) -> Result<settlement::SettlementValidatedArgs, String> {
        validate_params::validate_settlement_params(&self.ethereum_args, &self.starknet_args)
    }

    pub fn validate_prover_params(&self) -> Result<ProverValidatedArgs, String> {
        validate_params::validate_prover_params(&self.sharp_args, &self.atlantic_args)
    }

    pub fn validate_instrumentation_params(&self) -> Result<InstrumentationParams, String> {
        validate_params::validate_instrumentation_params(&self.instrumentation_args)
    }

    pub fn validate_server_params(&self) -> Result<ServerParams, String> {
        validate_params::validate_server_params(&self.server_args)
    }

    pub fn validate_service_params(&self) -> Result<ServiceParams, String> {
        validate_params::validate_service_params(&self.service_args)
    }

    pub fn validate_proving_layout_name(&self) -> Result<(LayoutName, LayoutName), String> {
        validate_params::validate_proving_layout_name(&self.proving_layout_args)
    }

    pub fn validate_snos_params(&self) -> Result<SNOSParams, String> {
        validate_params::validate_snos_params(&self.snos_args)
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[clap(
    group(
        ArgGroup::new("provider")
            .args(&["aws"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("storage")
            .args(&["aws_s3"])
            .required(true)
            .multiple(false)
            .requires("provider")
    ),
    group(
      ArgGroup::new("queue")
          .args(&["aws_sqs"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
      ArgGroup::new("alert")
          .args(&["aws_sns"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
        ArgGroup::new("cron")
            .args(&["aws_event_bridge"])
            .required(true)
            .multiple(false)
            .requires("provider")
    ),
)]
pub struct SetupCmd {
    // AWS Config
    #[clap(flatten)]
    pub aws_config_args: AWSConfigCliArgs,

    // Storage
    #[clap(flatten)]
    pub aws_s3_args: storage::aws_s3::AWSS3CliArgs,

    // Queue
    #[clap(flatten)]
    pub aws_sqs_args: queue::aws_sqs::AWSSQSCliArgs,

    // Alert
    #[clap(flatten)]
    pub aws_sns_args: alert::aws_sns::AWSSNSCliArgs,

    // Cron
    #[clap(flatten)]
    pub aws_event_bridge_args: AWSEventBridgeCliArgs,
}

impl SetupCmd {
    pub fn validate_provider_params(&self) -> Result<ProviderValidatedArgs, String> {
        validate_params::validate_provider_params(&self.aws_config_args)
    }

    pub fn validate_storage_params(&self) -> Result<StorageValidatedArgs, String> {
        validate_params::validate_storage_params(&self.aws_s3_args, &self.aws_config_args)
    }

    pub fn validate_queue_params(&self) -> Result<QueueValidatedArgs, String> {
        validate_params::validate_queue_params(&self.aws_sqs_args, &self.aws_config_args)
    }

    pub fn validate_alert_params(&self) -> Result<AlertValidatedArgs, String> {
        validate_params::validate_alert_params(&self.aws_sns_args, &self.aws_config_args)
    }

    pub fn validate_cron_params(&self) -> Result<CronValidatedArgs, String> {
        validate_params::validate_cron_params(&self.aws_event_bridge_args, &self.aws_config_args)
    }
}

pub mod validate_params {
    use std::str::FromStr as _;
    use std::time::Duration;

    use alloy::primitives::Address;
    use atlantic_service::AtlanticValidatedArgs;
    use cairo_vm::types::layout_name::LayoutName;
    use ethereum_da_client::EthereumDaValidatedArgs;
    use ethereum_settlement_client::EthereumSettlementValidatedArgs;
    use sharp_service::SharpValidatedArgs;
    use starknet_settlement_client::StarknetSettlementValidatedArgs;
    use url::Url;

    use super::alert::aws_sns::AWSSNSCliArgs;
    use super::alert::AlertValidatedArgs;
    use super::cron::event_bridge::AWSEventBridgeCliArgs;
    use super::cron::CronValidatedArgs;
    use super::da::ethereum::EthereumDaCliArgs;
    use super::da::DaValidatedArgs;
    use super::database::mongodb::MongoDBCliArgs;
    use super::database::DatabaseValidatedArgs;
    use super::instrumentation::InstrumentationCliArgs;
    use super::prover::atlantic::AtlanticCliArgs;
    use super::prover::sharp::SharpCliArgs;
    use super::prover::ProverValidatedArgs;
    use super::provider::aws::AWSConfigCliArgs;
    use super::provider::{AWSConfigValidatedArgs, ProviderValidatedArgs};
    use super::queue::aws_sqs::AWSSQSCliArgs;
    use super::queue::QueueValidatedArgs;
    use super::server::ServerCliArgs;
    use super::service::ServiceCliArgs;
    use super::settlement::ethereum::EthereumSettlementCliArgs;
    use super::settlement::starknet::StarknetSettlementCliArgs;
    use super::settlement::SettlementValidatedArgs;
    use super::snos::{SNOSCliArgs, SNOSParams};
    use super::storage::aws_s3::AWSS3CliArgs;
    use super::storage::StorageValidatedArgs;
    use crate::alerts::aws_sns::AWSSNSValidatedArgs;
    use crate::cli::prover_layout::ProverLayoutCliArgs;
    use crate::config::ServiceParams;
    use crate::cron::event_bridge::AWSEventBridgeValidatedArgs;
    use crate::data_storage::aws_s3::AWSS3ValidatedArgs;
    use crate::database::mongodb::MongoDBValidatedArgs;
    use crate::queue::sqs::AWSSQSValidatedArgs;
    use crate::routes::ServerParams;
    use crate::telemetry::InstrumentationParams;

    pub(crate) fn validate_provider_params(
        aws_config_args: &AWSConfigCliArgs,
    ) -> Result<ProviderValidatedArgs, String> {
        if aws_config_args.aws {
            Ok(ProviderValidatedArgs::AWS(AWSConfigValidatedArgs {
                aws_access_key_id: aws_config_args.aws_access_key_id.clone(),
                aws_secret_access_key: aws_config_args.aws_secret_access_key.clone(),
                aws_region: aws_config_args.aws_region.clone(),
            }))
        } else {
            Err("Only AWS is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_alert_params(
        aws_sns_args: &AWSSNSCliArgs,
        aws_config_args: &AWSConfigCliArgs,
    ) -> Result<AlertValidatedArgs, String> {
        if aws_sns_args.aws_sns && aws_config_args.aws {
            Ok(AlertValidatedArgs::AWSSNS(AWSSNSValidatedArgs {
                topic_arn: aws_sns_args.sns_arn.clone().expect("SNS ARN is required"),
            }))
        } else {
            Err("Only AWS SNS is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_queue_params(
        aws_sqs_args: &AWSSQSCliArgs,
        aws_config_args: &AWSConfigCliArgs,
    ) -> Result<QueueValidatedArgs, String> {
        if aws_sqs_args.aws_sqs && aws_config_args.aws {
            Ok(QueueValidatedArgs::AWSSQS(AWSSQSValidatedArgs {
                queue_base_url: Url::parse(&aws_sqs_args.queue_base_url.clone().expect("Queue base URL is required"))
                    .expect("Invalid queue base URL"),
                sqs_prefix: aws_sqs_args.sqs_prefix.clone().expect("SQS prefix is required"),
                sqs_suffix: aws_sqs_args.sqs_suffix.clone().expect("SQS suffix is required"),
            }))
        } else {
            Err("Only AWS SQS is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_storage_params(
        aws_s3_args: &AWSS3CliArgs,
        aws_config_args: &AWSConfigCliArgs,
    ) -> Result<StorageValidatedArgs, String> {
        if aws_s3_args.aws_s3 && aws_config_args.aws {
            Ok(StorageValidatedArgs::AWSS3(AWSS3ValidatedArgs {
                bucket_name: aws_s3_args.bucket_name.clone().expect("Bucket name is required"),
            }))
        } else {
            Err("Only AWS S3 is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_cron_params(
        aws_event_bridge_args: &AWSEventBridgeCliArgs,
        aws_config_args: &AWSConfigCliArgs,
    ) -> Result<CronValidatedArgs, String> {
        if aws_event_bridge_args.aws_event_bridge && aws_config_args.aws {
            Ok(CronValidatedArgs::AWSEventBridge(AWSEventBridgeValidatedArgs {
                target_queue_name: aws_event_bridge_args
                    .target_queue_name
                    .clone()
                    .expect("Target queue name is required"),
                cron_time: Duration::from_secs(
                    aws_event_bridge_args
                        .cron_time
                        .clone()
                        .expect("Cron time is required")
                        .parse::<u64>()
                        .expect("Failed to parse cron time"),
                ),
                trigger_rule_name: aws_event_bridge_args
                    .trigger_rule_name
                    .clone()
                    .expect("Trigger rule name is required"),

                trigger_role_name: aws_event_bridge_args
                    .trigger_role_name
                    .clone()
                    .expect("Trigger role name is required"),

                trigger_policy_name: aws_event_bridge_args
                    .trigger_policy_name
                    .clone()
                    .expect("Trigger policy name is required"),
            }))
        } else {
            Err("Only AWS Event Bridge is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_database_params(mongodb_args: &MongoDBCliArgs) -> Result<DatabaseValidatedArgs, String> {
        if mongodb_args.mongodb {
            Ok(DatabaseValidatedArgs::MongoDB(MongoDBValidatedArgs {
                connection_url: Url::parse(
                    &mongodb_args.mongodb_connection_url.clone().expect("MongoDB connection URL is required"),
                )
                .expect("Invalid MongoDB connection URL"),
                database_name: mongodb_args.mongodb_database_name.clone().expect("MongoDB database name is required"),
            }))
        } else {
            Err("Only MongoDB is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_da_params(ethereum_da_args: &EthereumDaCliArgs) -> Result<DaValidatedArgs, String> {
        if ethereum_da_args.da_on_ethereum {
            Ok(DaValidatedArgs::Ethereum(EthereumDaValidatedArgs {
                ethereum_da_rpc_url: ethereum_da_args
                    .ethereum_da_rpc_url
                    .clone()
                    .expect("Ethereum DA RPC URL is required"),
            }))
        } else {
            Err("Only Ethereum is supported as of now".to_string())
        }
    }

    pub(crate) fn validate_settlement_params(
        ethereum_args: &EthereumSettlementCliArgs,
        starknet_args: &StarknetSettlementCliArgs,
    ) -> Result<SettlementValidatedArgs, String> {
        match (ethereum_args.settle_on_ethereum, starknet_args.settle_on_starknet) {
            (true, true) => Err("Cannot settle on both Ethereum and Starknet".to_string()),
            (true, false) => {
                let l1_core_contract_address = Address::from_str(
                    &ethereum_args.l1_core_contract_address.clone().expect("L1 core contract address is required"),
                )
                .expect("Invalid L1 core contract address");
                let starknet_operator_address = Address::from_str(
                    &ethereum_args.starknet_operator_address.clone().expect("Starknet operator address is required"),
                )
                .expect("Invalid Starknet operator address");

                let ethereum_params = EthereumSettlementValidatedArgs {
                    ethereum_rpc_url: ethereum_args.ethereum_rpc_url.clone().expect("Ethereum RPC URL is required"),
                    ethereum_private_key: ethereum_args
                        .ethereum_private_key
                        .clone()
                        .expect("Ethereum private key is required"),
                    l1_core_contract_address,
                    starknet_operator_address,
                };
                Ok(SettlementValidatedArgs::Ethereum(ethereum_params))
            }
            (false, true) => {
                let starknet_params = StarknetSettlementValidatedArgs {
                    starknet_rpc_url: starknet_args.starknet_rpc_url.clone().expect("Starknet RPC URL is required"),
                    starknet_private_key: starknet_args
                        .starknet_private_key
                        .clone()
                        .expect("Starknet private key is required"),
                    starknet_account_address: starknet_args
                        .starknet_account_address
                        .clone()
                        .expect("Starknet account address is required"),
                    starknet_cairo_core_contract_address: starknet_args
                        .starknet_cairo_core_contract_address
                        .clone()
                        .expect("Starknet Cairo core contract address is required"),
                    starknet_finality_retry_wait_in_secs: starknet_args
                        .starknet_finality_retry_wait_in_secs
                        .expect("Starknet finality retry wait in seconds is required"),
                };
                Ok(SettlementValidatedArgs::Starknet(starknet_params))
            }
            (false, false) => Err("Settlement layer is required".to_string()),
        }
    }

    pub(crate) fn validate_prover_params(
        sharp_args: &SharpCliArgs,
        atlantic_args: &AtlanticCliArgs,
    ) -> Result<ProverValidatedArgs, String> {
        match (sharp_args.sharp, atlantic_args.atlantic) {
            (true, true) => Err("Cannot use both Sharp and Atlantic provers".to_string()),
            (true, false) => Ok(ProverValidatedArgs::Sharp(SharpValidatedArgs {
                sharp_customer_id: sharp_args.sharp_customer_id.clone().expect("Sharp customer ID is required"),
                sharp_url: sharp_args.sharp_url.clone().expect("Sharp URL is required"),
                sharp_user_crt: sharp_args.sharp_user_crt.clone().expect("Sharp user certificate is required"),
                sharp_user_key: sharp_args.sharp_user_key.clone().expect("Sharp user key is required"),
                sharp_rpc_node_url: sharp_args.sharp_rpc_node_url.clone().expect("Sharp RPC node URL is required"),
                sharp_proof_layout: sharp_args.sharp_proof_layout.clone().expect("Sharp proof layout is required"),
                gps_verifier_contract_address: sharp_args
                    .gps_verifier_contract_address
                    .clone()
                    .expect("GPS verifier contract address is required"),
                sharp_server_crt: sharp_args.sharp_server_crt.clone().expect("Sharp server certificate is required"),
            })),
            (false, true) => Ok(ProverValidatedArgs::Atlantic(AtlanticValidatedArgs {
                atlantic_api_key: atlantic_args.atlantic_api_key.clone().expect("Atlantic API key required"),
                atlantic_service_url: atlantic_args.atlantic_service_url.clone().expect("Atlantic URL is required"),
                atlantic_rpc_node_url: atlantic_args
                    .atlantic_rpc_node_url
                    .clone()
                    .expect("Atlantic API key is required"),
                atlantic_verifier_contract_address: atlantic_args
                    .atlantic_verifier_contract_address
                    .clone()
                    .expect("Atlantic verifier contract address is required"),
                atlantic_settlement_layer: atlantic_args
                    .atlantic_settlement_layer
                    .clone()
                    .expect("Atlantic settlement layer is required"),
                atlantic_mock_fact_hash: atlantic_args
                    .atlantic_mock_fact_hash
                    .clone()
                    .expect("Atlantic mock fact hash is required"),
                atlantic_prover_type: atlantic_args
                    .atlantic_prover_type
                    .clone()
                    .expect("Atlantic prover type is required"),
            })),
            (false, false) => Err("Prover is required".to_string()),
        }
    }

    pub(crate) fn validate_instrumentation_params(
        instrumentation_args: &InstrumentationCliArgs,
    ) -> Result<InstrumentationParams, String> {
        Ok(InstrumentationParams {
            otel_service_name: instrumentation_args.otel_service_name.clone().expect("Otel service name is required"),
            otel_collector_endpoint: instrumentation_args.otel_collector_endpoint.clone(),
        })
    }

    pub(crate) fn validate_server_params(server_args: &ServerCliArgs) -> Result<ServerParams, String> {
        Ok(ServerParams { host: server_args.host.clone(), port: server_args.port })
    }

    pub(crate) fn validate_proving_layout_name(args: &ProverLayoutCliArgs) -> Result<(LayoutName, LayoutName), String> {
        let layout_from_name = |layout_name: &str| -> Result<LayoutName, String> {
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
                _ => return Err(format!("Invalid layout name: {}", layout_name)),
            })
        };

        let prover_layout = layout_from_name(args.prover_layout_name.as_str())
            .map_err(|e| format!("Error validating prover layout: {}", e))?;
        let snos_layout = layout_from_name(args.snos_layout_name.as_str())
            .map_err(|e| format!("Error validating SNOS layout: {}", e))?;

        Ok((snos_layout, prover_layout))
    }

    pub(crate) fn validate_service_params(service_args: &ServiceCliArgs) -> Result<ServiceParams, String> {
        Ok(ServiceParams {
            // return None if the value is empty string
            max_block_to_process: service_args.max_block_to_process.clone().and_then(|s| {
                if s.is_empty() { None } else { Some(s.parse::<u64>().expect("Failed to parse max block to process")) }
            }),
            min_block_to_process: service_args.min_block_to_process.clone().and_then(|s| {
                if s.is_empty() { None } else { Some(s.parse::<u64>().expect("Failed to parse min block to process")) }
            }),
        })
    }

    pub(crate) fn validate_snos_params(snos_args: &SNOSCliArgs) -> Result<SNOSParams, String> {
        Ok(SNOSParams { rpc_for_snos: snos_args.rpc_for_snos.clone() })
    }

    #[cfg(test)]
    pub mod test {

        use rstest::rstest;
        use url::Url;

        use crate::cli::alert::aws_sns::AWSSNSCliArgs;
        use crate::cli::cron::event_bridge::AWSEventBridgeCliArgs;
        use crate::cli::da::ethereum::EthereumDaCliArgs;
        use crate::cli::database::mongodb::MongoDBCliArgs;
        use crate::cli::instrumentation::InstrumentationCliArgs;
        use crate::cli::prover::atlantic::AtlanticCliArgs;
        use crate::cli::prover::sharp::SharpCliArgs;
        use crate::cli::provider::aws::AWSConfigCliArgs;
        use crate::cli::queue::aws_sqs::AWSSQSCliArgs;
        use crate::cli::server::ServerCliArgs;
        use crate::cli::service::ServiceCliArgs;
        use crate::cli::settlement::ethereum::EthereumSettlementCliArgs;
        use crate::cli::settlement::starknet::StarknetSettlementCliArgs;
        use crate::cli::snos::SNOSCliArgs;
        use crate::cli::storage::aws_s3::AWSS3CliArgs;
        use crate::cli::validate_params::{
            validate_alert_params, validate_cron_params, validate_da_params, validate_database_params,
            validate_instrumentation_params, validate_prover_params, validate_provider_params, validate_queue_params,
            validate_server_params, validate_service_params, validate_settlement_params, validate_snos_params,
            validate_storage_params,
        };

        #[rstest]
        #[case(true)]
        #[case(false)]
        fn test_validate_provider_params(#[case] is_aws: bool) {
            let aws_config_args: AWSConfigCliArgs = AWSConfigCliArgs {
                aws: is_aws,
                aws_access_key_id: "".to_string(),
                aws_secret_access_key: "".to_string(),
                aws_region: "".to_string(),
            };

            let provider_params = validate_provider_params(&aws_config_args);
            if is_aws {
                assert!(provider_params.is_ok());
            } else {
                assert!(provider_params.is_err());
            }
        }

        #[rstest]
        #[case(true, true)]
        #[case(true, false)]
        #[case(false, true)]
        #[case(false, false)]
        fn test_validate_alert_params(#[case] is_aws: bool, #[case] is_sns: bool) {
            let aws_config_args: AWSConfigCliArgs = AWSConfigCliArgs {
                aws: is_aws,
                aws_access_key_id: "".to_string(),
                aws_secret_access_key: "".to_string(),
                aws_region: "".to_string(),
            };
            let aws_sns_args: AWSSNSCliArgs = AWSSNSCliArgs { aws_sns: is_sns, sns_arn: Some("".to_string()) };

            let alert_params = validate_alert_params(&aws_sns_args, &aws_config_args);
            if is_aws && is_sns {
                assert!(alert_params.is_ok());
            } else {
                assert!(alert_params.is_err());
            }
        }

        #[rstest]
        #[case(true, true)]
        #[case(true, false)]
        #[case(false, true)]
        #[case(false, false)]
        fn test_validate_queue_params(#[case] is_aws: bool, #[case] is_sqs: bool) {
            let aws_config_args: AWSConfigCliArgs = AWSConfigCliArgs {
                aws: is_aws,
                aws_access_key_id: "".to_string(),
                aws_secret_access_key: "".to_string(),
                aws_region: "".to_string(),
            };
            let aws_sqs_args: AWSSQSCliArgs = AWSSQSCliArgs {
                aws_sqs: is_sqs,
                queue_base_url: Some("http://sqs.us-east-1.localhost.localstack.cloud:4566/000000000000".to_string()),
                sqs_prefix: Some("".to_string()),
                sqs_suffix: Some("".to_string()),
            };
            let queue_params = validate_queue_params(&aws_sqs_args, &aws_config_args);
            if is_aws && is_sqs {
                assert!(queue_params.is_ok());
            } else {
                assert!(queue_params.is_err());
            }
        }

        #[rstest]
        #[case(true, true)]
        #[case(true, false)]
        #[case(false, true)]
        #[case(false, false)]
        fn test_validate_storage_params(#[case] is_aws: bool, #[case] is_s3: bool) {
            let aws_s3_args: AWSS3CliArgs = AWSS3CliArgs {
                aws_s3: is_s3,
                bucket_name: Some("".to_string()),
                bucket_location_constraint: Some("".to_string()),
            };
            let aws_config_args: AWSConfigCliArgs = AWSConfigCliArgs {
                aws: is_aws,
                aws_access_key_id: "".to_string(),
                aws_secret_access_key: "".to_string(),
                aws_region: "".to_string(),
            };
            let storage_params = validate_storage_params(&aws_s3_args, &aws_config_args);
            if is_aws && is_s3 {
                assert!(storage_params.is_ok());
            } else {
                assert!(storage_params.is_err());
            }
        }

        #[rstest]
        #[case(true)]
        #[case(false)]
        fn test_validate_database_params(#[case] is_mongodb: bool) {
            let mongodb_args: MongoDBCliArgs = MongoDBCliArgs {
                mongodb: is_mongodb,
                mongodb_connection_url: Some("mongodb://localhost:27017".to_string()),
                mongodb_database_name: Some("orchestrator".to_string()),
            };
            let database_params = validate_database_params(&mongodb_args);
            if is_mongodb {
                assert!(database_params.is_ok());
            } else {
                assert!(database_params.is_err());
            }
        }

        #[rstest]
        #[case(true)]
        #[case(false)]
        fn test_validate_da_params(#[case] is_ethereum: bool) {
            let ethereum_da_args: EthereumDaCliArgs = EthereumDaCliArgs {
                da_on_ethereum: is_ethereum,
                ethereum_da_rpc_url: Some(Url::parse("http://localhost:8545").unwrap()),
            };
            let da_params = validate_da_params(&ethereum_da_args);
            if is_ethereum {
                assert!(da_params.is_ok());
            } else {
                assert!(da_params.is_err());
            }
        }

        #[rstest]
        #[case(true, false)]
        #[case(false, true)]
        #[case(false, false)]
        #[case(true, true)]
        fn test_validate_settlement_params(#[case] is_ethereum: bool, #[case] is_starknet: bool) {
            let ethereum_args: EthereumSettlementCliArgs = EthereumSettlementCliArgs {
                ethereum_rpc_url: Some(Url::parse("http://localhost:8545").unwrap()),
                ethereum_private_key: Some("".to_string()),
                l1_core_contract_address: Some("0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057".to_string()),
                starknet_operator_address: Some("0x5b98B836969A60FEC50Fa925905Dd1D382a7db43".to_string()),
                settle_on_ethereum: is_ethereum,
            };
            let starknet_args: StarknetSettlementCliArgs = StarknetSettlementCliArgs {
                starknet_rpc_url: Some(Url::parse("http://localhost:8545").unwrap()),
                starknet_private_key: Some("".to_string()),
                starknet_account_address: Some("".to_string()),
                starknet_cairo_core_contract_address: Some("".to_string()),
                starknet_finality_retry_wait_in_secs: Some(0),
                settle_on_starknet: is_starknet,
            };
            let settlement_params = validate_settlement_params(&ethereum_args, &starknet_args);
            if is_ethereum ^ is_starknet {
                assert!(settlement_params.is_ok());
            } else {
                assert!(settlement_params.is_err());
            }
        }

        #[rstest]
        #[case(true, false)]
        #[case(false, true)]
        #[case(false, false)]
        #[case(true, true)]
        fn test_validate_prover_params(#[case] is_sharp: bool, #[case] is_atlantic: bool) {
            let sharp_args: SharpCliArgs = SharpCliArgs {
                sharp: is_sharp,
                sharp_customer_id: Some("".to_string()),
                sharp_url: Some(Url::parse("http://localhost:8545").unwrap()),
                sharp_user_crt: Some("".to_string()),
                sharp_user_key: Some("".to_string()),
                sharp_rpc_node_url: Some(Url::parse("http://localhost:8545").unwrap()),
                sharp_proof_layout: Some("".to_string()),
                gps_verifier_contract_address: Some("".to_string()),
                sharp_server_crt: Some("".to_string()),
            };

            let atlantic_args: AtlanticCliArgs = AtlanticCliArgs {
                atlantic: is_atlantic,
                atlantic_api_key: Some("".to_string()),
                atlantic_service_url: Some(Url::parse("http://localhost:8545").unwrap()),
                atlantic_rpc_node_url: Some(Url::parse("http://localhost:8545").unwrap()),
                atlantic_verifier_contract_address: Some("".to_string()),
                atlantic_settlement_layer: Some("".to_string()),
                atlantic_mock_fact_hash: Some("".to_string()),
                atlantic_prover_type: Some("".to_string()),
            };
            let prover_params = validate_prover_params(&sharp_args, &atlantic_args);
            if is_sharp ^ is_atlantic {
                assert!(prover_params.is_ok());
            } else {
                assert!(prover_params.is_err());
            }
        }

        #[rstest]
        #[case(true)]
        #[case(false)]
        fn test_validate_cron_params(#[case] is_aws: bool) {
            let aws_event_bridge_args: AWSEventBridgeCliArgs = AWSEventBridgeCliArgs {
                aws_event_bridge: is_aws,
                target_queue_name: Some(String::from("test")),
                cron_time: Some(String::from("12")),
                trigger_rule_name: Some(String::from("test")),
                trigger_role_name: Some(String::from("test-role")),
                trigger_policy_name: Some(String::from("test-policy")),
            };
            let aws_config_args: AWSConfigCliArgs = AWSConfigCliArgs {
                aws: is_aws,
                aws_access_key_id: "".to_string(),
                aws_secret_access_key: "".to_string(),
                aws_region: "".to_string(),
            };
            let cron_params = validate_cron_params(&aws_event_bridge_args, &aws_config_args);
            if is_aws {
                assert!(cron_params.is_ok());
            } else {
                assert!(cron_params.is_err());
            }
        }

        #[rstest]
        fn test_validate_instrumentation_params() {
            let instrumentation_args: InstrumentationCliArgs =
                InstrumentationCliArgs { otel_service_name: Some("".to_string()), otel_collector_endpoint: None };
            let instrumentation_params = validate_instrumentation_params(&instrumentation_args);
            assert!(instrumentation_params.is_ok());
        }

        #[rstest]
        fn test_validate_server_params() {
            let server_args: ServerCliArgs = ServerCliArgs { host: "".to_string(), port: 0 };
            let server_params = validate_server_params(&server_args);
            assert!(server_params.is_ok());
        }

        #[rstest]
        fn test_validate_snos_params() {
            let snos_args: SNOSCliArgs = SNOSCliArgs { rpc_for_snos: Url::parse("http://localhost:8545").unwrap() };
            let snos_params = validate_snos_params(&snos_args);
            assert!(snos_params.is_ok());
        }

        #[rstest]
        fn test_validate_service_params() {
            let service_args: ServiceCliArgs = ServiceCliArgs {
                max_block_to_process: Some("66645".to_string()),
                min_block_to_process: Some("100".to_string()),
            };
            let service_params = validate_service_params(&service_args);
            assert!(service_params.is_ok());
            let service_params = service_params.unwrap();
            assert_eq!(service_params.max_block_to_process, Some(66645));
            assert_eq!(service_params.min_block_to_process, Some(100));
        }
    }
}
