pub mod route;
pub mod types;
pub mod error;

use crate::core::config::Config;
use crate::params::service::ServerParams;
use crate::server::route::server_router;
use crate::OrchestratorResult;
use std::net::SocketAddr;
use std::sync::Arc;

/// Sets up and starts the HTTP server with configured routes.
///
/// This function:
/// 1. Initializes the server with the provided configuration
/// 2. Sets up all route handlers (both app and job routes)
/// 3. Starts the server in a separate tokio task
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `SocketAddr` - The bound address of the server
///
/// # Panics
/// * If the server fails to start
/// * If the address cannot be bound
pub async fn setup_server(config: Arc<Config>) -> OrchestratorResult<SocketAddr> {
    let (api_server_url, listener) = get_server_url(config.server_config()).await;

    let app = server_router(config.clone());
    // TODO:  Threading the server should not be here as it will block the main thread
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Failed to start axum server");
    });

    Ok(api_server_url)
}

async fn get_server_url(server_params: &ServerParams) -> (SocketAddr, tokio::net::TcpListener) {
    let address = format!("{}:{}", server_params.host, server_params.port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let api_server_url = listener.local_addr().expect("Unable to bind address to listener.");

    (api_server_url, listener)
}
