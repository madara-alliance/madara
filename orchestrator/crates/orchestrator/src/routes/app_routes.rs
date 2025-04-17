use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

/// Creates the main application router with basic health check and development routes.
///
/// This router provides fundamental application endpoints including:
/// - Health check endpoint at `/health`
/// - Development routes under `/v1/dev`
///
/// # Returns
/// * `Router` - Configured application router with health and dev routes
///
/// ```
pub fn app_router() -> Router {
    Router::new().route("/health", get(root)).nest("/v1/dev", dev_routes())
}

/// Health check endpoint handler.
///
/// Returns a simple "UP" response to indicate the service is running.
/// This endpoint is commonly used by load balancers and monitoring systems
/// to verify service availability.
///
/// # Returns
/// * `&'static str` - Always returns "UP"
///
/// ```
async fn root() -> &'static str {
    "UP"
}

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
    (StatusCode::NOT_FOUND, "The requested resource was not found")
}

/// Creates a router for development-only endpoints.
///
/// This router is nested under `/v1/dev` and is intended for
/// development and testing purposes. Currently empty but provides
/// a location for adding development-specific endpoints.
///
/// # Returns
/// * `Router` - Empty router for development endpoints
///
/// # Security
/// These routes should be disabled or properly secured in production environments.
fn dev_routes() -> Router {
    Router::new()
}
