use dotenvy::dotenv;
use orchestrator::config::init_config;
use orchestrator::queue::init_consumers;
use orchestrator::routes::app_router;
use orchestrator::telemetry::{setup_analytics, shutdown_analytics};

use utils::env_utils::get_env_var_or_default;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();

    // Analytics Setup

    let meter_provider = setup_analytics();

    // initial config setup
    let config = init_config().await;

    let host = get_env_var_or_default("HOST", "127.0.0.1");
    let port = get_env_var_or_default("PORT", "3000").parse::<u16>().expect("PORT must be a u16");
    let address = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let app = app_router();

    // init consumer
    init_consumers(config).await.expect("Failed to init consumers");

    tracing::info!("Listening on http://{}", address);
    axum::serve(listener, app).await.expect("Failed to start axum server");

    // Analytics Shutdown
    shutdown_analytics(meter_provider);
}
