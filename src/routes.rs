use crate::controllers::jobs_controller;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

pub fn app_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/health", get(root))
        .nest("/v1/dev", dev_routes(state.clone()))
        .nest("/v1/job", job_routes(state.clone()))
        .fallback(handler_404)
}

async fn root() -> &'static str {
    "UP"
}

async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "The requested resource was not found")
}

fn job_routes(state: AppState) -> Router<AppState> {
    Router::new().route("/create_job", post(jobs_controller::create_job)).with_state(state)
}

fn dev_routes(state: AppState) -> Router<AppState> {
    Router::new().with_state(state)
}
