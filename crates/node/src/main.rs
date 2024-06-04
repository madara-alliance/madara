//! Deoxys node command line.
#![warn(missing_docs)]

use anyhow::Context;
use clap::Parser;

#[macro_use]
mod service;
// mod benchmarking;
// mod chain_spec;
mod cli;
// mod command;
// mod commands;
// mod configs;
// mod genesis_block;
// mod rpc;
mod util;

use cli::RunCmd;
use service::{DatabaseService, RpcService, SyncService};
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    crate::util::setup_rayon_threadpool()?;
    let run_cmd: RunCmd = RunCmd::parse();

    let _db = DatabaseService::new(&run_cmd.db_params).context("initializing db service")?;
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
