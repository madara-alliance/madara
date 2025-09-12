//! Madara node command line.
#![warn(missing_docs)]

mod cli;
mod service;
mod submit_tx;
mod util;

use crate::service::L1SyncConfig;
use anyhow::{bail, Context};
use clap::Parser;
use cli::RunCmd;
use figment::{
    providers::{Format, Json, Serialized, Toml, Yaml},
    Figment,
};
use http::{HeaderName, HeaderValue};
use mc_analytics::Analytics;
use mc_db::DatabaseService;
use mc_gateway_client::GatewayProvider;
use mc_mempool::{Mempool, MempoolConfig};
use mc_settlement_client::gas_price::L1BlockMetrics;
use mc_submit_tx::{SubmitTransaction, TransactionValidator};
use mc_telemetry::{SysInfo, TelemetryService};
use mp_utils::service::{MadaraServiceId, ServiceMonitor};
use service::{BlockProductionService, GatewayService, L1SyncService, RpcService, SyncService, WarpUpdateConfig};
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::{env, path::Path};
use submit_tx::{MakeSubmitTransactionSwitch, MakeSubmitValidatedTransactionSwitch};

const GREET_IMPL_NAME: &str = "Madara";
const GREET_SUPPORT_URL: &str = "https://github.com/madara-alliance/madara/issues";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crate::util::setup_rayon_threadpool()?;
    crate::util::raise_fdlimit();

    // Create config builder.
    let mut config: Figment = Figment::new();

    // This loads the arguments in priority
    // If there are cli arguments, check if they are pointing to a file
    // If yes, load from that file. If not, load the values from the cli
    // If there are no cli args, load the default file
    if env::args().count() > 1 {
        // This is done to overwrite the preset with the args
        let cli_args = RunCmd::parse().apply_arg_preset();

        if let Some(config_path) = cli_args.config_file.clone() {
            config = match config_path.extension() {
                None => bail!("Unsupported file type for config file."),
                Some(os_str) => match os_str.to_str() {
                    Some("toml") => config.merge(Toml::file(config_path)),
                    Some("json") => config.merge(Json::file(config_path)),
                    Some("yaml") => config.merge(Yaml::file(config_path)),
                    _ => bail!("Unsupported file type for config file."),
                },
            }
        } else {
            config = config.merge(Serialized::defaults(cli_args));
        }
    } else {
        let path = Path::new("./configs/args/config.json");
        if path.exists() {
            config = config.merge(Json::file(path));
        }
    }

    // Extracts the arguments into the struct
    let mut run_cmd: RunCmd = config.extract()?;
    run_cmd.check_mode()?;

    // Setting up analytics

    let mut analytics = Analytics::new(
        run_cmd.analytics_params.analytics_service_name.clone(),
        run_cmd.analytics_params.analytics_collection_endpoint.clone(),
    )
    .context("Initializing analytics service")?;
    analytics.setup()?;

    // If it's a sequencer or a devnet we set the mandatory chain config. If it's a full node we set the chain config from the network or the custom chain config.
    let chain_config = if run_cmd.is_sequencer() {
        run_cmd.chain_config()?
    } else if run_cmd.network.is_some() {
        run_cmd.set_preset_from_network()?
    } else {
        run_cmd.chain_config()?
    };

    // If block time is inferior to the tick time, then only empty blocks will
    // be produced as we will never update the pending block before storing it.
    if run_cmd.is_sequencer() && chain_config.pending_block_update_time.is_some_and(|t| chain_config.block_time < t) {
        anyhow::bail!(
            "Block time ({:?}) cannot be less than the pending block update time ({:?}), as this will yield only empty blocks",
            chain_config.block_time,
            chain_config.pending_block_update_time.expect("Condition already checked")
        );
    }

    // If the devnet is running, we set the gas prices to a default value.
    if run_cmd.is_devnet() {
        run_cmd.l1_sync_params.l1_sync_disabled = true;
        run_cmd.l1_sync_params.l1_gas_price.get_or_insert(128);
        run_cmd.l1_sync_params.blob_gas_price.get_or_insert(128);
        run_cmd.l1_sync_params.strk_per_eth.get_or_insert(1.0);
    }

    // Check if the devnet is running with the correct chain id. This is purely
    // to avoid accidental setups which would allow for replay attacks. This is
    // possible if the devnet has the same chain id as another popular chain,
    // allowing txs which occur on it to also be replayed on that other chain.
    if run_cmd.devnet
        && (chain_config.chain_id == ChainId::Mainnet || chain_config.chain_id == ChainId::Sepolia)
        && !run_cmd.devnet_unsafe
    {
        anyhow::bail!("You're running a devnet with the network config of {0}. This means that devnet transactions can be replayed on the actual {0} network. Use `--network=devnet` instead or force this configuration with `--devnet-unsafe`.", chain_config.chain_name);
    }

    let node_name = run_cmd.node_name_or_provide().await.to_string();
    let node_version = env!("MADARA_BUILD_VERSION");

    tracing::info!("🥷 {} Node", GREET_IMPL_NAME);
    tracing::info!("✌️  Version {}", node_version);
    tracing::info!("💁 Support URL: {}", GREET_SUPPORT_URL);
    tracing::info!("🏷  Node Name: {}", node_name);
    let role = if run_cmd.is_sequencer() { "Sequencer" } else { "Full Node" };
    tracing::info!("👤 Role: {}", role);
    tracing::info!("🌐 Network: {} (chain id `{}`)", chain_config.chain_name, chain_config.chain_id);
    run_cmd.args_preset.greet();

    let sys_info = SysInfo::probe();
    sys_info.show();

    // ===================================================================== //
    //                             SERVICES (SETUP)                          //
    // ===================================================================== //

    // Telemetry

    let service_telemetry: TelemetryService =
        TelemetryService::new(run_cmd.telemetry_params.telemetry_endpoints.clone())
            .context("Initializing telemetry service")?;

    // Database

    let service_db = DatabaseService::new(chain_config.clone(), run_cmd.db_params.backend_config())
        .await
        .context("Initializing db service")?;

    // declare mempool here so that it can be used to process l1->l2 messages in the l1 service
    let mempool = Mempool::new(
        Arc::clone(service_db.backend()),
        MempoolConfig { save_to_db: !run_cmd.validator_params.no_mempool_saving },
    );
    mempool.load_txs_from_db().await.context("Loading mempool transactions")?;
    let mempool = Arc::new(mempool);

    let (l1_head_snd, l1_head_recv) = tokio::sync::watch::channel(None);
    let service_l1_sync = L1SyncService::new(
        &run_cmd.l1_sync_params,
        service_db.backend().clone(),
        L1SyncConfig {
            l1_core_address: chain_config.eth_core_contract_address.clone(),
            authority: run_cmd.is_sequencer(),
            l1_block_metrics: L1BlockMetrics::register().context("Initializing L1 Block Metrics")?.into(),
            l1_head_snd,
        },
    )
    .await
    .context("Initializing l1 sync service")?;

    // L2 Sync

    let warp_update = if run_cmd.args_preset.warp_update_receiver {
        let mut deferred_service_start = vec![];
        let mut deferred_service_stop = vec![];

        if !run_cmd.rpc_params.rpc_disable {
            deferred_service_start.push(MadaraServiceId::RpcUser);
        }

        if run_cmd.rpc_params.rpc_admin {
            deferred_service_start.push(MadaraServiceId::RpcAdmin);
        }

        if run_cmd.gateway_params.any_enabled() {
            deferred_service_start.push(MadaraServiceId::Gateway);
        }

        if run_cmd.telemetry_params.telemetry {
            deferred_service_start.push(MadaraServiceId::Telemetry);
        }

        if run_cmd.is_sequencer() {
            deferred_service_start.push(MadaraServiceId::BlockProduction);
            deferred_service_stop.push(MadaraServiceId::L2Sync);
        }

        Some(WarpUpdateConfig {
            warp_update_port_rpc: run_cmd.l2_sync_params.warp_update_port_rpc,
            warp_update_port_fgw: run_cmd.l2_sync_params.warp_update_port_fgw,
            warp_update_shutdown_sender: run_cmd.l2_sync_params.warp_update_shutdown_sender,
            warp_update_shutdown_receiver: run_cmd.l2_sync_params.warp_update_shutdown_receiver,
            deferred_service_start,
            deferred_service_stop,
        })
    } else {
        None
    };

    let service_l2_sync = SyncService::new(&run_cmd.l2_sync_params, service_db.backend(), l1_head_recv, warp_update)
        .await
        .context("Initializing sync service")?;

    let mut provider = GatewayProvider::new(chain_config.gateway_url.clone(), chain_config.feeder_gateway_url.clone());

    // gateway api key is needed for declare transactions on mainnet
    if let Some(url) = run_cmd.validator_params.validate_then_forward_txs_to.clone() {
        provider = provider.with_madara_gateway_url(url)
    }
    if let Some(api_key) = run_cmd.l2_sync_params.gateway_key.clone() {
        provider.add_header(
            HeaderName::from_static("x-throttling-bypass"),
            HeaderValue::from_str(&api_key).with_context(|| "Invalid API key format")?,
        )
    }

    let gateway_client = Arc::new(provider);

    // Block production

    let service_block_production = BlockProductionService::new(
        &run_cmd.block_production_params,
        &service_db,
        Arc::clone(&mempool),
        service_l1_sync.client(),
    )?;

    // Add transaction provider

    let mempool_tx_validator = Arc::new(TransactionValidator::new(
        Arc::clone(&mempool) as _,
        Arc::clone(service_db.backend()),
        run_cmd.validator_params.as_validator_config(),
    ));

    let gateway_submit_tx: Arc<dyn SubmitTransaction> =
        if run_cmd.validator_params.validate_then_forward_txs_to.is_some() {
            Arc::new(TransactionValidator::new(
                Arc::clone(&gateway_client) as _,
                Arc::clone(service_db.backend()),
                run_cmd.validator_params.as_validator_config(),
            ))
        } else {
            Arc::clone(&gateway_client) as _
        };

    let tx_submit =
        MakeSubmitTransactionSwitch::new(Arc::clone(&gateway_submit_tx) as _, Arc::clone(&mempool_tx_validator) as _);
    let validated_tx_submit =
        MakeSubmitValidatedTransactionSwitch::new(Arc::clone(&gateway_client) as _, Arc::clone(&mempool) as _);

    // User-facing RPC

    let service_rpc_user = RpcService::user(
        run_cmd.rpc_params.clone(),
        Arc::clone(service_db.backend()),
        tx_submit.clone(),
        service_l1_sync.provider(),
    );

    // Admin-facing RPC (for node operators)

    let service_rpc_admin = RpcService::admin(
        run_cmd.rpc_params.clone(),
        Arc::clone(service_db.backend()),
        tx_submit.clone(),
        service_block_production.handle(),
    );

    // Feeder gateway

    let service_gateway = GatewayService::new(
        run_cmd.gateway_params.clone(),
        Arc::clone(service_db.backend()),
        tx_submit.clone(),
        Some(validated_tx_submit.clone()),
    )
    .await
    .context("Initializing gateway service")?;

    service_telemetry.send_connected(&node_name, node_version, &chain_config.chain_name, &sys_info);

    // ===================================================================== //
    //                             SERVICES (START)                          //
    // ===================================================================== //

    if run_cmd.is_devnet() {
        service_block_production.setup_devnet().await?;
    }

    let app = ServiceMonitor::default()
        .with(service_db)?
        .with(service_l1_sync)?
        .with(service_l2_sync)?
        .with(service_block_production)?
        .with(service_rpc_user)?
        .with(service_rpc_admin)?
        .with(service_gateway)?
        .with(service_telemetry)?;

    // Since the database is not implemented as a proper service, we do not
    // active it, as it would never be marked as stopped by the existing logic
    //
    // app.activate(MadaraService::Database);

    let l1_sync_enabled = !run_cmd.l1_sync_params.l1_sync_disabled;
    let l1_endpoint_some = run_cmd.l1_sync_params.l1_endpoint.is_some();
    let warp_update_receiver = run_cmd.args_preset.warp_update_receiver;

    if l1_sync_enabled && (l1_endpoint_some || !run_cmd.devnet) {
        app.activate(MadaraServiceId::L1Sync);
    }

    if warp_update_receiver {
        app.activate(MadaraServiceId::L2Sync);
    } else if run_cmd.is_sequencer() {
        app.activate(MadaraServiceId::BlockProduction);
    } else if !run_cmd.l2_sync_params.l2_sync_disabled {
        app.activate(MadaraServiceId::L2Sync);
    }

    if !run_cmd.rpc_params.rpc_disable && !warp_update_receiver {
        app.activate(MadaraServiceId::RpcUser);
    }

    if run_cmd.rpc_params.rpc_admin && !warp_update_receiver {
        app.activate(MadaraServiceId::RpcAdmin);
    }

    if run_cmd.gateway_params.any_enabled() && !warp_update_receiver {
        app.activate(MadaraServiceId::Gateway);
    }

    if run_cmd.telemetry_params.telemetry && !warp_update_receiver {
        app.activate(MadaraServiceId::Telemetry);
    }

    app.start().await
}
