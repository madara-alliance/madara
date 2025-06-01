use axum::routing::get;
use axum::Router;

pub(super) fn local_route() -> Router {
    Router::new().route("/health", get(health_checker_handler))
}

async fn health_checker_handler() -> &'static str {
    "UP"
}
