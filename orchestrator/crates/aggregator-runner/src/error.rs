#[derive(Debug, thiserror::Error)]
pub enum AggregatorRunnerError {
    #[error("Aggregator execution failed: {0}")]
    AggregatorExecution(String),

    #[error("CairoPIE validity check failed: {0}")]
    PieValidation(String),

    #[error("Failed to create temp file for DA segment: {0}")]
    TempFileCreation(#[from] std::io::Error),

    #[error("Failed to read DA segment from temp file: {0}")]
    DaSegmentRead(String),

    #[error("No child program outputs provided")]
    NoChildOutputs,
}
