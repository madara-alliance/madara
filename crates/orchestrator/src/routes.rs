use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use crate::controllers::jobs_controller;

pub fn app_router() -> Router {
    Router::new()
        .route("/health", get(root))
        .nest("/v1/dev", dev_routes())
        .nest("/v1/job", job_routes())
        .fallback(handler_404)
}

async fn root() -> &'static str {
    "UP"
}

async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "The requested resource was not found")
}

fn job_routes() -> Router {
    Router::new().route("/create_job", post(jobs_controller::create_job))
}

fn dev_routes() -> Router {
    Router::new()
}
