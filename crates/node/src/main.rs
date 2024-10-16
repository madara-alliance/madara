//! Madara node command line.
#![warn(missing_docs)]
#![warn(clippy::unwrap_used)]

mod cli;
mod service;
mod util;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use cli::{NetworkType, RunCmd};
use mc_analytics::Analytics;
use mc_block_import::BlockImporter;
use mc_db::DatabaseService;
use mc_mempool::{GasPriceProvider, L1DataProvider, Mempool};
use mc_rpc::providers::{AddTransactionProvider, ForwardToProvider, MempoolAddTxProvider};
use mc_telemetry::{SysInfo, TelemetryService};
use mp_convert::ToFelt;
use mp_utils::service::{Service, ServiceGroup};
use service::{BlockProductionService, GatewayService, L1SyncService, RpcService, SyncService};
use starknet_providers::SequencerGatewayProvider;

const GREET_IMPL_NAME: &str = "Madara";
const GREET_SUPPORT_URL: &str = "https://github.com/madara-alliance/madara/issues";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // crate::util::setup_logging()?;
    crate::util::setup_rayon_threadpool()?;
    crate::util::raise_fdlimit();

    let mut run_cmd: RunCmd = RunCmd::parse();

    // Setting up analytics

    let mut analytics = Analytics::new(
        run_cmd.analytics_params.analytics_service_name.clone(),
        run_cmd.analytics_params.analytics_log_level.clone(),
        run_cmd.analytics_params.analytics_collection_endpoint.clone(),
    )
    .context("Initializing analytics service")?;
    analytics.setup().unwrap();

    // If it's a sequencer or a devnet we set the mandatory chain config. If it's a full node we set the chain config from the network or the custom chain config.
    let chain_config = if run_cmd.is_sequencer() {
        run_cmd.chain_config()?
    } else if run_cmd.network.is_some() {
        run_cmd.set_preset_from_network()?
    } else {
        run_cmd.chain_config()?
    };

    let node_name = run_cmd.node_name_or_provide().await.to_string();
    let node_version = env!("DEOXYS_BUILD_VERSION");

    log::info!("🥷  {} Node", GREET_IMPL_NAME);
    log::info!("✌️  Version {}", node_version);
    log::info!("💁 Support URL: {}", GREET_SUPPORT_URL);
    log::info!("🏷  Node Name: {}", node_name);
    let role = if run_cmd.is_sequencer() { "Sequencer" } else { "Full Node" };
    log::info!("👤 Role: {}", role);
    log::info!("🌐 Network: {} (chain id `{}`)", chain_config.chain_name, chain_config.chain_id);

    let sys_info = SysInfo::probe();
    sys_info.show();

    // Services.

    // TODO: analytics service is the first one to get registered now, no need to make it a service now, just use setup like logger.
    let telemetry_service: TelemetryService =
        TelemetryService::new(run_cmd.telemetry_params.telemetry, run_cmd.telemetry_params.telemetry_endpoints.clone())
            .context("Initializing telemetry service")?;

    let db_service = DatabaseService::new(
        &run_cmd.db_params.base_path,
        run_cmd.db_params.backup_dir.clone(),
        run_cmd.db_params.restore_from_latest_backup,
        Arc::clone(&chain_config),
    )
    .await
    .context("Initializing db service")?;

    let importer = Arc::new(
        BlockImporter::new(
            Arc::clone(db_service.backend()),
            run_cmd.sync_params.unsafe_starting_block,
            // Always flush when in authority mode as we really want to minimize the risk of losing a block when the app is unexpectedly killed :)
            /* always_force_flush */
            run_cmd.is_sequencer(),
        )
        .context("Initializing importer service")?,
    );

    let l1_gas_setter = GasPriceProvider::new();
    let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());
    if run_cmd.devnet {
        run_cmd.l1_sync_params.sync_l1_disabled = true;
        run_cmd.l1_sync_params.gas_price_sync_disabled = true;
    }

    let l1_service = L1SyncService::new(
        &run_cmd.l1_sync_params,
        &db_service,
        l1_gas_setter,
        chain_config.chain_id.clone(),
        chain_config.eth_core_contract_address,
        run_cmd.is_sequencer(),
    )
    .await
    .context("Initializing the l1 sync service")?;

    // Block provider startup.
    // `rpc_add_txs_method_provider` is a trait object that tells the RPC task where to put the transactions when using the Write endpoints.
    let (block_provider_service, rpc_add_txs_method_provider): (_, Arc<dyn AddTransactionProvider>) =
        match run_cmd.is_sequencer() {
            // Block production service. (authority)
            true => {
                let mempool = Arc::new(Mempool::new(Arc::clone(db_service.backend()), Arc::clone(&l1_data_provider)));

                let block_production_service = BlockProductionService::new(
                    &run_cmd.block_production_params,
                    &db_service,
                    Arc::clone(&mempool),
                    importer,
                    Arc::clone(&l1_data_provider),
                    run_cmd.devnet,
                    telemetry_service.new_handle(),
                )?;

                (ServiceGroup::default().with(block_production_service), Arc::new(MempoolAddTxProvider::new(mempool)))
            }
            // Block sync service. (full node)
            false => {
                // Feeder gateway sync service.
                let sync_service = SyncService::new(
                    &run_cmd.sync_params,
                    Arc::clone(&chain_config),
                    run_cmd.network.context(
                        "You should provide a `--network` argument to ensure you're syncing from the right FGW",
                    )?,
                    &db_service,
                    importer,
                    telemetry_service.new_handle(),
                )
                .await
                .context("Initializing sync service")?;

                (
                ServiceGroup::default().with(sync_service),
                // TODO(rate-limit): we may get rate limited with this unconfigured provider?
                Arc::new(ForwardToProvider::new(SequencerGatewayProvider::new(
                    run_cmd
                        .network
                        .context(
                            "You should provide a `--network` argument to ensure you're syncing from the right gateway",
                        )?
                        .gateway(),
                    run_cmd
                        .network
                        .context(
                            "You should provide a `--network` argument to ensure you're syncing from the right FGW",
                        )?
                        .feeder_gateway(),
                    chain_config.chain_id.to_felt(),
                ))),
            )
            }
        };

    let rpc_service = RpcService::new(
        &run_cmd.rpc_params,
        &db_service,
        Arc::clone(&chain_config),
        Arc::clone(&rpc_add_txs_method_provider),
    )
    .context("Initializing rpc service")?;

    let gateway_service = GatewayService::new(&run_cmd.gateway_params, &db_service, rpc_add_txs_method_provider)
        .await
        .context("Initializing gateway service")?;

    telemetry_service.send_connected(&node_name, node_version, &chain_config.chain_name, &sys_info);

    let app = ServiceGroup::default()
        .with(db_service)
        .with(l1_service)
        .with(block_provider_service)
        .with(rpc_service)
        .with(gateway_service)
        .with(telemetry_service);

    // Check if the devnet is running with the correct chain id.
    if run_cmd.devnet && chain_config.chain_id != NetworkType::Devnet.chain_id() {
        if !run_cmd.block_production_params.override_devnet_chain_id {
            log::error!("You're running a devnet with the network config of {:?}. This means that devnet transactions can be replayed on the actual network. Use `--network=devnet` instead. Or if this is the expected behavior please pass `--override-devnet-chain-id`", chain_config.chain_name);
            panic!();
        } else {
            // This log is immediately flooded with devnet accounts and so this can be missed.
            // Should we add a delay here to make this clearly visisble?
            log::warn!("You're running a devnet with the network config of {:?}. This means that devnet transactions can be replayed on the actual network.", run_cmd.network);
        }
    }

    app.start_and_drive_to_end().await?;

    tracing::info!("Shutting down analytics");
    analytics.shutdown();

    Ok(())
}
