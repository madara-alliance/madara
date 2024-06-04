//! Deoxys node command line.
#![warn(missing_docs)]

use anyhow::Context;
use clap::Parser;

mod service;
mod cli;
mod util;

use cli::RunCmd;
use service::{DatabaseService, RpcService, SyncService};
use tokio::task::JoinSet;

const GREET_IMPL_NAME: &'static str = "Deoxys";
const GREET_SUPPORT_URL: &'static str =  "https://kasar.io";
const GREET_AUTHORS: &[&'static str] = &[
  "Kasar <https://github.com/kasarlabs>",
  "KSS <https://github.com/keep-starknet-strange>",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    crate::util::setup_rayon_threadpool()?;
    let mut run_cmd: RunCmd = RunCmd::parse();
    run_cmd.provide_node_name().await;

    log::info!("👽 {} Node", GREET_IMPL_NAME);
    log::info!("✌️  version {}", env!("DEOXYS_BUILD_VERSION"));
    for author in GREET_AUTHORS {
        log::info!("❤️  by {}", author);
    }
    log::info!("💁 Support URL: {}", GREET_SUPPORT_URL);
    log::info!("👤 Role: full node");
    log::info!("🏷  Node name: {}", run_cmd.name.unwrap());

    let _db = DatabaseService::open(&run_cmd.db_params).context("initializing db service")?;
    let mut rpc = RpcService::new(&run_cmd.rpc_params, run_cmd.sync_params.network).context("initializing rpc service")?;
    let mut sync_service = SyncService::new(&run_cmd.sync_params, None).context("initializing sync service")?;

    let mut task_set = JoinSet::new();
    sync_service.start(&mut task_set).await.context("starting sync service")?;
    rpc.start(&mut task_set).await.context("starting rpc service")?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        result = task_set.join_next() => {
            if let Some(result) = result {
                result.context("tokio join error")??
            }
        }
    };

    Ok(())
}
