pub mod constants;

use rstest::*;
use constants::*;
use std::env;

use orchestrator::config::{config, Config};

#[fixture]
pub async fn get_or_init_config(
    #[default(MADARA_RPC_URL.to_string())]rpc_url: String,
    #[default(DA_LAYER.to_string())]da_layer: String,
    #[default(MONGODB_CONNECTION_STRING.to_string())]mongo_url: String,
) -> &'static Config {
    env::set_var("MADARA_RPC_URL", rpc_url);
    env::set_var("DA_LAYER", da_layer);
    env::set_var("MONGODB_CONNECTION_STRING", mongo_url);
    
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    config().await
}
