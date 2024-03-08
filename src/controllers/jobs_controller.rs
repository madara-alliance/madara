use crate::controllers::errors::AppError;
use crate::jobs::types::JobType;
use crate::AppState;
use axum::extract::Json;
use axum::extract::State;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    job_type: JobType,
    internal_id: String,
}

pub async fn create_job(
    State(_state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<Json<()>, AppError> {
    crate::jobs::create_job(payload.job_type, payload.internal_id).await?;
    Ok(Json::from(()))
}
