/// Config of the service. Contains configurations for DB, Queues and other services.
mod config;
/// Controllers for the routes
mod controllers;
/// Contains the trait that all database clients must implement
mod database;
/// Contains the trait that all jobs must implement. Also
/// contains the root level functions for which detect the job
/// type and call the corresponding job
mod jobs;
/// Contains the trait that all queues must implement
mod queue;
/// Contains the routes for the service
mod routes;
/// Contains the utils
mod utils;

use crate::config::config;
use crate::queue::init_consumers;
use crate::routes::app_router;
use crate::utils::env_utils::get_env_var_or_default;
use dotenvy::dotenv;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).init();

    // initial config setup
    config().await;
    let host = get_env_var_or_default("HOST", "127.0.0.1");
    let port = get_env_var_or_default("PORT", "3000").parse::<u16>().expect("PORT must be a u16");
    let address = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let app = app_router();

    // init consumer
    init_consumers().await.expect("Failed to init consumers");

    tracing::info!("Listening on http://{}", address);
    axum::serve(listener, app).await.expect("Failed to start axum server");
}
