use axum::response::IntoResponse;
use axum::Json;
use color_eyre::eyre::ErrReport;
use serde_json::json;
use tracing::log;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Internal Server Error {0}")]
    InternalServerError(#[from] ErrReport),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::http::Response<axum::body::Body> {
        log::error!("Error: {:?}", self);
        let (status, err_msg) = match self {
            Self::InternalServerError(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg.to_string()),
        };
        (status, Json(json!({"message": err_msg }))).into_response()
    }
}
