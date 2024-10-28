use std::net::SocketAddr;
use std::sync::Arc;

use app_routes::{app_router, handler_404};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use job_routes::job_router;
use serde::Serialize;
use utils::env_utils::get_env_var_or_default;

use crate::config::Config;

pub mod app_routes;
pub mod job_routes;

#[derive(Debug, Serialize)]
struct ApiResponse<T>
where
    T: Serialize,
{
    data: Option<T>,
    error: Option<String>,
}

impl<T> ApiResponse<T>
where
    T: Serialize,
{
    pub fn success(data: T) -> Self {
        Self { data: Some(data), error: None }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self { data: None, error: Some(message.into()) }
    }
}

impl<T> IntoResponse for ApiResponse<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        let status = if self.error.is_some() { StatusCode::INTERNAL_SERVER_ERROR } else { StatusCode::OK };

        let json = Json(self);

        (status, json).into_response()
    }
}

pub async fn setup_server(config: Arc<Config>) -> SocketAddr {
    let (api_server_url, listener) = get_server_url().await;

    let job_routes = job_router(config.clone());
    let app_routes = app_router();
    let app = Router::new().merge(app_routes).merge(job_routes).fallback(handler_404);

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Failed to start axum server");
    });

    api_server_url
}

pub async fn get_server_url() -> (SocketAddr, tokio::net::TcpListener) {
    let host = get_env_var_or_default("HOST", "127.0.0.1");
    let port = get_env_var_or_default("PORT", "3000").parse::<u16>().expect("PORT must be a u16");
    let address = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let api_server_url = listener.local_addr().expect("Unable to bind address to listener.");

    (api_server_url, listener)
}
