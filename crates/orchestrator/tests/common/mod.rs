pub mod constants;

use rstest::*;
use constants::*;
use std::env;

use orchestrator::config::{config, Config};

#[fixture]
pub async fn init_valid_config(
    #[default(MADARA_RPC_URL.to_string())]rpcurl: String
) -> &'static Config {
    env::set_var("MADARA_RPC_URL", rpcurl);
    env::set_var("MONGODB_CONNECTION_STRING", MONGODB_CONNECTION_STRING);
    
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    config().await
}
