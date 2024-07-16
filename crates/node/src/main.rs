//! Deoxys node command line.
#![warn(missing_docs)]

use anyhow::Context;
use clap::Parser;

mod cli;
mod service;
mod util;

use cli::RunCmd;
use dc_db::DatabaseService;
use dc_metrics::MetricsService;
use dc_telemetry::{SysInfo, TelemetryService};
use service::{RpcService, SyncService};
use tokio::task::JoinSet;

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
    log::info!("👤 Role: Full Node");
    log::info!("🌐 Network: {}", network_name);

    let sys_info = SysInfo::probe();
    sys_info.show();

    let mut telemetry_service = TelemetryService::new(
        run_cmd.telemetry_params.telemetry_disabled,
        run_cmd.telemetry_params.telemetry_endpoints.clone(),
    )
    .context("Initializing telemetry service")?;
    let mut prometheus_service = MetricsService::new(
        run_cmd.prometheus_params.prometheus_disabled,
        run_cmd.prometheus_params.prometheus_external,
        run_cmd.prometheus_params.prometheus_port,
    )
    .context("Initializing prometheus metrics service")?;

    let db = DatabaseService::new(
        &run_cmd.db_params.base_path,
        run_cmd.db_params.backup_dir.clone(),
        run_cmd.db_params.restore_from_latest_backup,
        &run_cmd.sync_params.network.db_chain_info(),
    )
    .await
    .context("Initializing db service")?;
    let mut rpc = RpcService::new(&run_cmd.rpc_params, &db, run_cmd.sync_params.network, prometheus_service.registry())
        .context("Initializing rpc service")?;
    let mut sync_service =
        SyncService::new(&run_cmd.sync_params, &db, prometheus_service.registry(), telemetry_service.new_handle())
            .await
            .context("Initializing sync service")?;

    let mut task_set = JoinSet::new();

    sync_service.start(&mut task_set).await.context("Starting sync service")?;
    rpc.start(&mut task_set).await.context("Starting rpc service")?;
    telemetry_service.start(&mut task_set).await.context("Starting telemetry service")?;
    prometheus_service.start(&mut task_set).await.context("Starting prometheus metrics service")?;

    telemetry_service.send_connected(
        &node_name,
        node_version,
        &run_cmd.sync_params.network.db_chain_info().chain_name,
        &sys_info,
    );

    while let Some(result) = task_set.join_next().await {
        // Ignore tokio join errors, only bubble service errors
        if let Ok(err) = result {
            err?
        }
    }

    Ok(())
}
