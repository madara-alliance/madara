use crate::core::config::Config;
use alloy::transports::http::reqwest::StatusCode;
use axum::response::IntoResponse;
use axum::Router;
use jobs::job_router;
use public::local_route;
use std::sync::Arc;

pub(super) mod public;
pub(super) mod jobs;

/// Handles 404 Not Found responses for the application.
///
/// This handler is used as a fallback when no other routes match the request.
/// It provides a consistent error response format across the application.
///
/// # Returns
/// * `impl IntoResponse` - Returns a 404 status code with a descriptive message
///
/// # Examples
/// ```
/// // When accessing an undefined route:
/// // GET /undefined -> 404 Not Found
/// // Response: "The requested resource was not found"
/// ```
pub async fn handler_404() -> impl IntoResponse {
    // TODO: when running the server, it is always recommended to use the JSON format
    //  for the response. However, when running the server locally, it is recommended to use the
    //  plain text format. This is because the local server is not running in a browser, so the
    //  browser will not be able to display the JSON response.
    (StatusCode::NOT_FOUND, "The requested resource was not found")
}

fn v1_route(config: Arc<Config>) -> Router {
    Router::new()
        .nest("/jobs", job_router(config))
}

pub(crate) fn server_router(config: Arc<Config>) -> Router {
    let v1_routes = Router::new().nest("/v1", v1_route(config));
    let routes = Router::new()
        .nest("/", local_route())
        .nest("/api", v1_routes)
        .nest("/jobs", job_router(config))
        .fallback(handler_404);
    routes
}
