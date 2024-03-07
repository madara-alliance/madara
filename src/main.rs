mod config;
mod da_clients;
mod database;
mod jobs;
mod queue;
mod routes;
mod utils;
mod controllers;

use crate::config::config;
use crate::queue::init_consumers;
use crate::routes::app_router;
use crate::utils::env_utils::get_env_var_or_default;
use dotenvy::dotenv;

#[derive(Clone)]
pub struct AppState {}

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
    let state = AppState {};
    let app = app_router(state.clone()).with_state(state);

    // init consumer
    init_consumers().await.expect("Failed to init consumers");

    tracing::info!("Listening on http://{}", address);
    axum::serve(listener, app).await.expect("Failed to start axum server");
}
