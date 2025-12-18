pub mod error;
pub mod middleware;
pub mod route;
pub mod service;
pub mod types;

use crate::core::config::Config;
use crate::types::params::service::ServerParams;
use crate::{server::route::server_router, OrchestratorResult};
use std::net::SocketAddr;
use std::sync::Arc;

// re-export axum macros
pub use error::{ApiServiceError, ApiServiceResult};

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
    tokio::spawn(async move { axum::serve(listener, app).await.expect("Failed to start axum server") });

    Ok(api_server_url)
}

pub(crate) async fn get_server_url(server_params: &ServerParams) -> (SocketAddr, tokio::net::TcpListener) {
    // In test mode, use port 0 to get a random available port (prevents "Address already in use" errors)
    // In production, use the configured port
    let port = if cfg!(test) {
        0 // Let OS assign an available port
    } else {
        server_params.port
    };

    let address = format!("{}:{}", server_params.host, port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let api_server_url = listener.local_addr().expect("Unable to bind address to listener.");

    (api_server_url, listener)
}
