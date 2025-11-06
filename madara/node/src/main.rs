//! Madara is a hybrid RPC node / sequencer for the [Starknet] network. As of the time of writing,
//! it is the only node with full support for both state synchronization and state production in
//! this way. Madara is designed to be performant, flexible and observable for your use, whether it
//! be in a dev environment or a production-ready custom chain.
//!
//! This file serves as the entry point into the node software, which is organized into many
//! different crates. The crates are separated into three different categories, made explicit by
//! their own naming convention.
//!
//! # Specs
//!
//! The Madara node follows the [Starknet RPC specs] as well as the [Starknet P2P specs] in the
//! implementations of its various components. Where seems fit, some extensions might be added to
//! the specs, in particular in regards to any potential admin or privileged endpoints which might
//! prove to be useful in prod.
//!
//! # Core Crates
//! > `mc-*`
//!
//! These are crates which are responsible for the core (business) functionality of the node. They
//! answer the question of _how_ the node transforms data.
//!
//! - [mc-analytics]
//! - [mc-block-production]
//! - [mc-db]
//! - [mc-devnet]
//! - [mc-exec]
//! - [mc-gateway-client]
//! - [mc-gateway-server]
//! - [mc-mempool]
//! - [mc-rpc]
//! - [mc-settlement-client]
//! - [mc-submit-tx]
//! - [mc-sync]
//! - [mc-telemetry]
//! - [mc-e2e-tests]
//!
//! # Primitives Crates
//! > `mp-*`
//!
//! Crates which are responsible for modelling the data in use by the node. These mostly don't
//! contain any business logic (though some helper functions might be included) and instead focus on
//! data types as well as serialization and deserialization. These crates answer the question of
//! _what_ data the node transforms.
//!
//! - [mp-block]
//! - [mp-bloom-filter]
//! - [mp-chain-config]
//! - [mp-class]
//! - [mp-convert]
//! - [mp-gateway]
//! - [mp-oracle]
//! - [mp-receipt]
//! - [mp-rpc]
//! - [mp-state-update]
//! - [mp-transactions]
//! - [mp-utils]
//!
//! # Helper Crates
//!
//! These are external crates with no real business logic or types definitions. They are used either
//! in testing contexts or to make the developer's life easier with domain-specific macros.
//!
//! - [m-cairo-test-contracts]
//! - [m-proc-macros]
//!
//! [Starknet]: https://www.starknet.io
//! [Starknet RPC specs]: https://github.com/starkware-libs/starknet-specs
//! [Starknet P2P specs]: https://github.com/starknet-io/starknet-p2p-specs
//!
//! [mc-analytics]: mc_analytics
//! [mc-block-production]: mc_block_production
//! [mc-db]: mc_db
//! [mc-devnet]: mc_devnet
//! [mc-exec]: mc_exec
//! [mc-gateway-client]: mc_gateway_client
//! [mc-gateway-server]: mc_gateway_server
//! [mc-mempool]: mc_mempool
//! [mc-rpc]: mc_rpc
//! [mc-settlement-client]: mc_settlement_client
//! [mc-submit-tx]: mc_submit_tx
//! [mc-sync]: mc_sync
//! [mc-telemetry]: mc_telemetry
//! [mc-e2e-tests]: mc_e2e_tests
//!
//! [mp-block]: mp_block
//! [mp-bloom-filter]: mp_bloom_filter
//! [mp-chain-config]: mp_chain_config
//! [mp-class]: mp_class
//! [mp-convert]: mp_convert
//! [mp-gateway]: mp_gateway
//! [mp-oracle]: mp_oracle
//! [mp-receipt]: mp_receipt
//! [mp-rpc]: mp_rpc
//! [mp-state-update]: mp_state_update
//! [mp-transactions]: mp_transactions
//! [mp-utils]: mp_utils
//!
//! [m-cairo-test-contracts]: m_cairo_test_contracts
//! [m-proc-macros]: m_proc_macros
#![warn(missing_docs)]

use mc_db::MadaraStorageRead;
mod cli;
mod service;
mod submit_tx;
mod util;

use crate::service::{L1SyncConfig, MempoolService};
use anyhow::{bail, Context};
use clap::Parser;
use cli::RunCmd;
use dotenv::dotenv;
use figment::{
    providers::{Format, Json, Serialized, Toml, Yaml},
    Figment,
};
use http::{HeaderName, HeaderValue};
use mc_analytics::AnalyticsService;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
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
    dotenv().ok();
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
    let mut service_analytics = AnalyticsService::new(run_cmd.analytics_params.as_analytics_config())
        .context("Initializing analytics service")?;
    service_analytics.setup().context("Setting-up analystics service")?;

    // If it's a sequencer or a devnet we set the mandatory chain config. If it's a full node we set the chain config from the network or the custom chain config.
    let chain_config = if run_cmd.is_sequencer() {
        run_cmd.chain_config()?
    } else if run_cmd.network.is_some() {
        run_cmd.set_preset_from_network()?
    } else {
        run_cmd.chain_config()?
    };

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

    tracing::info!("🥷  {} Node", GREET_IMPL_NAME);
    tracing::info!("💁 Support URL: {}", GREET_SUPPORT_URL);
    tracing::info!("🏷  Node Name: {}", node_name);
    let role = if run_cmd.is_sequencer() { "Sequencer" } else { "Full Node" };
    tracing::info!("👤 Role: {}", role);
    tracing::info!("🌐 Network: {} (chain id `{}`)", chain_config.chain_name, chain_config.chain_id);
    run_cmd.args_preset.greet();

    let sys_info = SysInfo::probe();
    sys_info.show();

    // Initialize Cairo Native configuration
    //
    // The configuration is validated and passed through parameters (no global state).
    // Native execution is opt-in and only enabled when the --enable-native-execution CLI flag
    // is set to true (default: false). When disabled, all contracts will use Cairo VM execution
    // regardless of cache state.
    let cairo_native_config = run_cmd.cairo_native_params.to_runtime_config();

    // Setup: validate, initialize semaphore, and log configuration
    // Note: Validation happens inside setup_and_log() via NativeConfig::validate()
    mc_cairo_native::config::setup_and_log(&cairo_native_config)
        .map_err(|e| anyhow::anyhow!("Cairo Native configuration setup failed: {}", e))?;

    // ===================================================================== //
    //                             SERVICES (SETUP)                          //
    // ===================================================================== //

    // Telemetry

    let service_telemetry: TelemetryService =
        TelemetryService::new(run_cmd.telemetry_params.telemetry_endpoints.clone())
            .context("Initializing telemetry service")?;

    // Database

    tracing::info!("💾 Opening database at: {}", run_cmd.backend_params.base_path.display());
    let cairo_native_config_arc = Arc::new(cairo_native_config);
    let backend = MadaraBackend::open_rocksdb(
        &run_cmd.backend_params.base_path,
        chain_config.clone(),
        run_cmd.backend_params.backend_config(),
        run_cmd.backend_params.rocksdb_config(),
        cairo_native_config_arc.clone(),
    )
    .context("Starting madara backend")?;

    let chain_tip = backend.db.get_chain_tip().expect("Chain tip should have been fetched.");
    tracing::info!("💼 Starting chain with block: {:?}", chain_tip);

    let service_mempool = MempoolService::new(&run_cmd, backend.clone());

    let (l1_head_snd, l1_head_recv) = tokio::sync::watch::channel(None);
    let service_l1_sync = L1SyncService::new(
        &run_cmd.l1_sync_params,
        backend.clone(),
        L1SyncConfig {
            l1_core_address: chain_config.eth_core_contract_address.clone(),
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

    let service_l2_sync = SyncService::new(
        &run_cmd.l2_sync_params,
        &backend,
        l1_head_recv,
        warp_update,
        /* unsafe_starting_block_enabled */ run_cmd.backend_params.unsafe_starting_block.is_some(),
    )
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
        &backend,
        service_mempool.mempool(),
        service_l1_sync.client(),
        run_cmd.validator_params.no_charge_fee,
    )?;

    // Add transaction provider

    let mempool_tx_validator = Arc::new(TransactionValidator::new(
        service_mempool.mempool() as _,
        backend.clone(),
        run_cmd.validator_params.as_validator_config(),
    ));

    let gateway_submit_tx: Arc<dyn SubmitTransaction> =
        if run_cmd.validator_params.validate_then_forward_txs_to.is_some() {
            Arc::new(TransactionValidator::new(
                Arc::clone(&gateway_client) as _,
                backend.clone(),
                run_cmd.validator_params.as_validator_config(),
            ))
        } else {
            Arc::clone(&gateway_client) as _
        };

    let tx_submit =
        MakeSubmitTransactionSwitch::new(Arc::clone(&gateway_submit_tx) as _, Arc::clone(&mempool_tx_validator) as _);
    let validated_tx_submit =
        MakeSubmitValidatedTransactionSwitch::new(Arc::clone(&gateway_client) as _, service_mempool.mempool() as _);

    // User-facing RPC

    let service_rpc_user = RpcService::user(run_cmd.rpc_params.clone(), backend.clone(), tx_submit.clone());

    // Admin-facing RPC (for node operators)

    let service_rpc_admin = RpcService::admin(
        run_cmd.rpc_params.clone(),
        backend.clone(),
        tx_submit.clone(),
        service_block_production.handle(),
    );

    // Feeder gateway

    let service_gateway = GatewayService::new(
        run_cmd.gateway_params.clone(),
        backend.clone(),
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
        .with(service_analytics)?
        .with(service_mempool)?
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

    app.activate(MadaraServiceId::Mempool);
    app.activate(MadaraServiceId::Analytics);

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
