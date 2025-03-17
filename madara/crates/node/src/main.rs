//! Madara node command line.
#![warn(missing_docs)]

mod cli;
mod service;
mod util;

use anyhow::{bail, Context};
use clap::Parser;
use cli::RunCmd;
use http::{HeaderName, HeaderValue};
use mc_analytics::Analytics;
use mc_block_import::BlockImporter;
use mc_db::{DatabaseService, TrieLogConfig};
use mc_gateway_client::GatewayProvider;
use mc_mempool::{GasPriceProvider, L1DataProvider, Mempool, MempoolLimits};
use mc_rpc::providers::{AddTransactionProvider, ForwardToProvider, MempoolAddTxProvider};
use mc_sync::fetch::fetchers::WarpUpdateConfig;
use mc_telemetry::{SysInfo, TelemetryService};
use mp_oracle::pragma::PragmaOracleBuilder;
use mp_sync::SyncStatusProvider;
use mp_utils::service::{MadaraServiceId, ServiceMonitor};
use service::{BlockProductionService, GatewayService, L1SyncService, L2SyncService, RpcService};
use starknet_api::core::ChainId;
use std::sync::Arc;

const GREET_IMPL_NAME: &str = "Madara";
const GREET_SUPPORT_URL: &str = "https://github.com/madara-alliance/madara/issues";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crate::util::setup_rayon_threadpool()?;
    crate::util::raise_fdlimit();

    let mut run_cmd = RunCmd::parse().apply_arg_preset();

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
    if run_cmd.is_sequencer() && chain_config.block_time < chain_config.pending_block_update_time {
        anyhow::bail!(
            "Block time ({}s) cannot be less than the pending block update time ({}s), as this will yield only empty blocks",
            chain_config.block_time.as_secs(),
            chain_config.pending_block_update_time.as_secs()
        );
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
    tracing::info!("🏷 Node Name: {}", node_name);
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

    let service_db = DatabaseService::new(
        &run_cmd.db_params.base_path,
        run_cmd.db_params.backup_dir.clone(),
        run_cmd.db_params.restore_from_latest_backup,
        Arc::clone(&chain_config),
        TrieLogConfig {
            max_saved_trie_logs: run_cmd.db_params.db_max_saved_trie_logs,
            max_kept_snapshots: run_cmd.db_params.db_max_kept_snapshots,
            snapshot_interval: run_cmd.db_params.db_snapshot_interval,
        },
    )
    .await
    .context("Initializing db service")?;

    // L1 Sync

    let mut l1_gas_setter = GasPriceProvider::new();

    if let Some(fix_gas) = run_cmd.l1_sync_params.gas_price {
        l1_gas_setter.update_eth_l1_gas_price(fix_gas as u128);
        l1_gas_setter.set_gas_price_sync_enabled(false);
    }
    if let Some(fix_blob_gas) = run_cmd.l1_sync_params.blob_gas_price {
        l1_gas_setter.update_eth_l1_data_gas_price(fix_blob_gas as u128);
        l1_gas_setter.set_data_gas_price_sync_enabled(false);
    }
    if let Some(strk_fix_gas) = run_cmd.l1_sync_params.strk_gas_price {
        l1_gas_setter.update_strk_l1_gas_price(strk_fix_gas as u128);
        l1_gas_setter.set_strk_gas_price_sync_enabled(false);
    }
    if let Some(strk_fix_blob_gas) = run_cmd.l1_sync_params.strk_blob_gas_price {
        l1_gas_setter.update_strk_l1_data_gas_price(strk_fix_blob_gas as u128);
        l1_gas_setter.set_strk_data_gas_price_sync_enabled(false);
    }
    if let Some(ref oracle_url) = run_cmd.l1_sync_params.oracle_url {
        if let Some(ref oracle_api_key) = run_cmd.l1_sync_params.oracle_api_key {
            let oracle = PragmaOracleBuilder::new()
                .with_api_url(oracle_url.clone())
                .with_api_key(oracle_api_key.clone())
                .build();
            l1_gas_setter.set_oracle_provider(oracle);
        }
    }

    if !run_cmd.full
        && !run_cmd.devnet
        && !run_cmd.l1_sync_params.l1_sync_disabled
        && l1_gas_setter.is_oracle_needed()
        && l1_gas_setter.oracle_provider.is_none()
    {
        bail!("STRK gas is not fixed and oracle is not provided");
    }

    let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

    // declare mempool here so that it can be used to process l1->l2 messages in the l1 service
    let mut mempool = Mempool::new(
        Arc::clone(service_db.backend()),
        Arc::clone(&l1_data_provider),
        MempoolLimits::new(&chain_config),
    );
    mempool.load_txs_from_db().context("Loading mempool transactions")?;
    let mempool = Arc::new(mempool);

    let service_l1_sync = L1SyncService::new(
        &run_cmd.l1_sync_params,
        &service_db,
        l1_gas_setter,
        chain_config.chain_id.clone(),
        chain_config.eth_core_contract_address,
        run_cmd.is_sequencer(),
        run_cmd.is_devnet(),
        Arc::clone(&mempool),
    )
    .await
    .context("Initializing the l1 sync service")?;

    // L2 Sync

    let importer = Arc::new(
        BlockImporter::new(Arc::clone(service_db.backend()), run_cmd.l2_sync_params.unsafe_starting_block)
            .context("Initializing importer service")?,
    );

    let warp_update = if run_cmd.args_preset.warp_update_receiver {
        let mut deferred_service_start = vec![];
        let mut deferred_service_stop = vec![];

        if !run_cmd.rpc_params.rpc_disable {
            deferred_service_start.push(MadaraServiceId::RpcUser);
        }

        if run_cmd.rpc_params.rpc_admin {
            deferred_service_start.push(MadaraServiceId::RpcAdmin);
        }

        if run_cmd.gateway_params.feeder_gateway_enable {
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

    let sync_status_provider = Arc::new(SyncStatusProvider::new());
    // fix: create a mutex arc of sync status, that would have information like starting block, current synced block and block till what we have to sync
    let service_l2_sync = L2SyncService::new(
        &run_cmd.l2_sync_params,
        Arc::clone(&chain_config),
        &service_db,
        importer,
        service_telemetry.new_handle(),
        warp_update,
        Arc::clone(&sync_status_provider),
    )
    .await
    .context("Initializing sync service")?;

    let mut provider = GatewayProvider::new(chain_config.gateway_url.clone(), chain_config.feeder_gateway_url.clone());

    // gateway api key is needed for declare transactions on mainnet
    if let Some(api_key) = run_cmd.l2_sync_params.gateway_key.clone() {
        provider.add_header(
            HeaderName::from_static("x-throttling-bypass"),
            HeaderValue::from_str(&api_key).with_context(|| "Invalid API key format")?,
        )
    }

    // Block production

    let importer = Arc::new(
        BlockImporter::new(Arc::clone(service_db.backend()), run_cmd.l2_sync_params.unsafe_starting_block)
            .context("Initializing importer service")?,
    );
    let service_block_production = BlockProductionService::new(
        &run_cmd.block_production_params,
        &service_db,
        Arc::clone(&mempool),
        importer,
        Arc::clone(&l1_data_provider),
    )?;

    // Add transaction provider
    let add_tx_provider_l2_sync: Arc<dyn AddTransactionProvider> = Arc::new(ForwardToProvider::new(provider));
    let add_tx_provider_mempool: Arc<dyn AddTransactionProvider> = Arc::new(MempoolAddTxProvider::new(mempool));

    // User-facing RPC

    let service_rpc_user = RpcService::user(
        run_cmd.rpc_params.clone(),
        Arc::clone(service_db.backend()),
        Arc::clone(&add_tx_provider_l2_sync),
        Arc::clone(&add_tx_provider_mempool),
        Arc::clone(&sync_status_provider),
    );

    // Admin-facing RPC (for node operators)

    let service_rpc_admin = RpcService::admin(
        run_cmd.rpc_params.clone(),
        Arc::clone(service_db.backend()),
        Arc::clone(&add_tx_provider_l2_sync),
        Arc::clone(&add_tx_provider_mempool),
        Arc::clone(&sync_status_provider),
    );

    // Feeder gateway

    let service_gateway = GatewayService::new(
        run_cmd.gateway_params.clone(),
        Arc::clone(service_db.backend()),
        Arc::clone(&add_tx_provider_l2_sync),
        Arc::clone(&add_tx_provider_mempool),
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

    if run_cmd.gateway_params.feeder_gateway_enable && !warp_update_receiver {
        app.activate(MadaraServiceId::Gateway);
    }

    if run_cmd.telemetry_params.telemetry && !warp_update_receiver {
        app.activate(MadaraServiceId::Telemetry);
    }

    app.start().await?;

    let _ = analytics.shutdown();

    anyhow::Ok(())
}
