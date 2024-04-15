#[cfg(test)]
pub mod controllers;

#[cfg(test)]
pub mod database;

#[cfg(test)]
pub mod jobs;

#[cfg(test)]
pub mod server;

#[cfg(test)]
pub mod queue;

pub mod common;

use orchestrator::config::{config, Config};

use std::env;
use rstest::*;
use crate::common::constants::{MADARA_RPC_URL, MONGODB_CONNECTION_STRING};

#[fixture]
fn rpc_url() -> String {
    String::from(MADARA_RPC_URL)
}

#[fixture]
pub async fn init_valid_config(
    rpc_url: String
) -> &'static Config {
    env::set_var("MADARA_RPC_URL", rpc_url.as_str());
    env::set_var("MONGODB_CONNECTION_STRING", MONGODB_CONNECTION_STRING);
    
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    config().await
}
