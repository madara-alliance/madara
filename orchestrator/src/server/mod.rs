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
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

// re-export axum macros
pub use error::{ApiServiceError, ApiServiceResult};

/// Handle for managing the HTTP server lifecycle.
pub struct ServerHandle {
    shutdown_token: CancellationToken,
    task_handle: JoinHandle<()>,
}

impl ServerHandle {
    /// Initiates graceful shutdown and waits for the server to stop.
    ///
    /// This will:
    /// 1. Signal the server to stop accepting new connections
    /// 2. Wait for in-flight requests to complete
    /// 3. Return when the server has fully stopped
    pub async fn shutdown(self) -> Result<(), tokio::task::JoinError> {
        info!("Initiating server graceful shutdown");
        self.shutdown_token.cancel();
        self.task_handle.await
    }
}

/// Sets up and starts the HTTP server with configured routes.
///
/// This function:
/// 1. Initializes the server with the provided configuration
/// 2. Sets up all route handlers (both app and job routes)
/// 3. Starts the server in a separate tokio task with graceful shutdown support
///
/// # Arguments
/// * `config` - Shared application configuration
///
/// # Returns
/// * `(SocketAddr, ServerHandle)` - The bound address and handle for managing the server
///
/// # Panics
/// * If the server fails to start
/// * If the address cannot be bound
pub async fn setup_server(config: Arc<Config>) -> OrchestratorResult<(SocketAddr, ServerHandle)> {
    let (api_server_url, listener) = get_server_url(config.server_config()).await;

    let shutdown_token = CancellationToken::new();
    let server_token = shutdown_token.clone();

    let app = server_router(config.clone());
    let task_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_token.cancelled_owned())
            .await
            .expect("Failed to start axum server")
    });

    let handle = ServerHandle { shutdown_token, task_handle };

    Ok((api_server_url, handle))
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
