use mongodb::bson::{self, Bson, Document};
use serde::Serialize;
use std::future::Future;
use std::time::Instant;

use crate::core::client::database::error::DatabaseError;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use opentelemetry::KeyValue;

/// Trait for converting types to BSON Document
pub trait ToDocument {
    fn to_document(&self) -> Result<Document, DatabaseError>;
}

impl<T: Serialize> ToDocument for T {
    fn to_document(&self) -> Result<Document, DatabaseError> {
        let bson = bson::to_bson(self)?;
        match bson {
            Bson::Document(doc) => Ok(doc),
            _ => Err(DatabaseError::FailedToSerializeDocument("Expected document, got other BSON type".to_string())),
        }
    }
}

/// Record metrics for a database operation
pub async fn record_metrics<T, F, Fut>(operation: &'static str, f: F) -> Result<T, DatabaseError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, DatabaseError>>,
{
    let start = Instant::now();
    let result = f().await;
    let duration = start.elapsed();

    let attributes = [KeyValue::new("db_operation_name", operation)];
    ORCHESTRATOR_METRICS.db_calls_response_time.record(duration.as_secs_f64(), &attributes);

    result
}
