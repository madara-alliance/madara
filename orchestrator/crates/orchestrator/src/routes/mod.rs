use std::net::SocketAddr;
use std::sync::Arc;

use app_routes::{app_router, handler_404};
use axum::Router;
use job_routes::job_router;

use crate::config::Config;

/// Routes module for the orchestrator service.
///
/// This module provides the core routing and server setup functionality, organizing
/// different route handlers into submodules:
/// - `app_routes`: General application routes (e.g., health checks)
/// - `job_routes`: Job processing and management routes
/// - `error`: Error handling and HTTP response mapping
/// - `types`: Shared type definitions for route handlers
pub mod app_routes;
pub mod error;
pub mod job_routes;
pub mod types;

pub use error::JobRouteError;
use crate::cli::ServerParams;

/// Configuration parameters for the HTTP server.
///
/// Contains the necessary information to bind and start the server.
///
/// # Examples
/// ```
/// let params = ServerParams { host: "127.0.0.1".to_string(), port: 8080 };
/// ```
// #[derive(Debug, Clone)]
// pub struct ServerParams {
//     /// The host address to bind to (e.g., "127.0.0.1", "0.0.0.0")
//     pub host: String,
//     /// The port number to listen on
//     pub port: u16,
// }

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
///
/// # Examples
/// ```
/// let config = Arc::new(Config::new());
/// let addr = setup_server(config).await;
/// println!("Server listening on {}", addr);
/// ```
pub async fn setup_server(config: Arc<Config>) -> SocketAddr {
    let (api_server_url, listener) = get_server_url(config.server_config()).await;

    let job_routes = job_router(config.clone());
    let app_routes = app_router();
    let app = Router::new().merge(app_routes).merge(job_routes).fallback(handler_404);

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Failed to start axum server");
    });

    api_server_url
}

/// Creates a TCP listener and returns its address.
///
/// This function handles the low-level socket binding and address resolution.
///
/// # Arguments
/// * `server_params` - Configuration for the server binding
///
/// # Returns
/// * `(SocketAddr, TcpListener)` - The bound address and the TCP listener
///
/// # Panics
/// * If binding to the specified address fails
/// * If the listener cannot be created
///
/// # Examples
/// ```
/// let params = ServerParams { host: "127.0.0.1".to_string(), port: 8080 };
/// let (addr, listener) = get_server_url(&params).await;
/// ```
pub async fn get_server_url(server_params: &ServerParams) -> (SocketAddr, tokio::net::TcpListener) {
    let address = format!("{}:{}", server_params.host, server_params.port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let api_server_url = listener.local_addr().expect("Unable to bind address to listener.");

    (api_server_url, listener)
}
