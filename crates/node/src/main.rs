//! Deoxys node command line.
#![warn(missing_docs)]

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
mod cli;
mod service;
mod util;

use crate::service::L1SyncService;
use cli::RunCmd;
use dc_db::DatabaseService;
use dc_mempool::{GasPriceProvider, L1DataProvider, Mempool};
use dc_metrics::MetricsService;
use dc_rpc::providers::AddTransactionProvider;
use dc_telemetry::{SysInfo, TelemetryService};
use dp_convert::ToFelt;
use dp_utils::service::{Service, ServiceGroup};
use service::{BlockProductionService, RpcService, SyncService};
use starknet_providers::SequencerGatewayProvider;
const GREET_IMPL_NAME: &str = "Deoxys";
const GREET_SUPPORT_URL: &str = "https://github.com/KasarLabs/deoxys/issues";
const GREET_AUTHORS: &[&str] = &["KasarLabs <https://kasar.io>"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crate::util::setup_logging()?;
    crate::util::setup_rayon_threadpool()?;
    crate::util::raise_fdlimit();

    let mut run_cmd: RunCmd = RunCmd::parse();
    let node_name = run_cmd.node_name_or_provide().await.to_string();
    let network_name = run_cmd.network().await.to_string();
    let node_version = env!("DEOXYS_BUILD_VERSION");

    log::info!("👽 {} Node", GREET_IMPL_NAME);
    log::info!("✌️  Version {}", node_version);
    for author in GREET_AUTHORS {
        log::info!("❤️  By {}", author);
    }
    log::info!("💁 Support URL: {}", GREET_SUPPORT_URL);
    log::info!("🏷  Node Name: {}", node_name);
    let role = if run_cmd.authority { "authority" } else { "full node" };
    log::info!("👤 Role: {}", role);
    log::info!("🌐 Network: {}", network_name);

    let sys_info = SysInfo::probe();
    sys_info.show();

    // Services.

    let telemetry_service = TelemetryService::new(
        run_cmd.telemetry_params.telemetry_disabled,
        run_cmd.telemetry_params.telemetry_endpoints.clone(),
    )
    .context("Initializing telemetry service")?;
    let prometheus_service = MetricsService::new(
        run_cmd.prometheus_params.prometheus_disabled,
        run_cmd.prometheus_params.prometheus_external,
        run_cmd.prometheus_params.prometheus_port,
    )
    .context("Initializing prometheus metrics service")?;

    let db_service = DatabaseService::new(
        &run_cmd.db_params.base_path,
        run_cmd.db_params.backup_dir.clone(),
        run_cmd.db_params.restore_from_latest_backup,
        run_cmd.sync_params.network.db_chain_info(),
    )
    .await
    .context("Initializing db service")?;

    let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(GasPriceProvider::new());

    let l1_service = L1SyncService::new(
        &run_cmd.l1_sync_params,
        &db_service,
        prometheus_service.registry(),
        Arc::clone(&l1_data_provider),
        run_cmd.sync_params.network.chain_id(),
        run_cmd.sync_params.network.l1_core_address(),
    )
    .await
    .context("Initializing the l1 sync service")?;

    // Block provider startup.
    // `rpc_add_txs_method_provider` is a trait object that tells the RPC task where to put the transactions when using the Write endpoints.
    let (block_provider_service, rpc_add_txs_method_provider): (_, Arc<dyn AddTransactionProvider>) =
        match run_cmd.authority {
            // Block production service. (authority)
            true => {
                let mempool = Arc::new(Mempool::new(Arc::clone(db_service.backend()), Arc::clone(&l1_data_provider)));

                let block_production_service = BlockProductionService::new(
                    &run_cmd.block_production_params,
                    &db_service,
                    Arc::clone(&mempool),
                    Arc::clone(&l1_data_provider),
                    prometheus_service.registry(),
                    telemetry_service.new_handle(),
                )?;

                (
                    ServiceGroup::default().with(block_production_service),
                    Arc::new(dc_rpc::mempool_provider::MempoolProvider::new(mempool)),
                )
            }
            // Block sync service. (full node)
            false => {
                // Feeder gateway sync service.
                let sync_service = SyncService::new(
                    &run_cmd.sync_params,
                    &db_service,
                    prometheus_service.registry(),
                    telemetry_service.new_handle(),
                )
                .await
                .context("Initializing sync service")?;

                (
                    ServiceGroup::default().with(sync_service),
                    // TODO(rate-limit): we may get rate limited with this unconfigured provider?
                    Arc::new(dc_rpc::providers::ForwardToProvider::new(SequencerGatewayProvider::new(
                        run_cmd.sync_params.network.gateway(),
                        run_cmd.sync_params.network.feeder_gateway(),
                        run_cmd.sync_params.network.chain_id().to_felt(),
                    ))),
                )
            }
        };

    let rpc_service = RpcService::new(
        &run_cmd.rpc_params,
        &db_service,
        run_cmd.sync_params.network,
        prometheus_service.registry(),
        rpc_add_txs_method_provider,
    )
    .context("Initializing rpc service")?;

    telemetry_service.send_connected(
        &node_name,
        node_version,
        &run_cmd.sync_params.network.db_chain_info().chain_name,
        &sys_info,
    );

    let app = ServiceGroup::default()
        .with(db_service)
        .with(l1_service)
        .with(block_provider_service)
        .with(rpc_service)
        .with(telemetry_service)
        .with(prometheus_service);

    app.start_and_drive_to_end().await?;
    Ok(())
}
