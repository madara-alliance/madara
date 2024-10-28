use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

pub fn app_router() -> Router {
    Router::new().route("/health", get(root)).nest("/v1/dev", dev_routes())
}

async fn root() -> &'static str {
    "UP"
}

pub async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "The requested resource was not found")
}

fn dev_routes() -> Router {
    Router::new()
}
