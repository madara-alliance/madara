use cairo_vm::types::builtin_name::BuiltinName;

#[derive(Debug, thiserror::Error)]
pub enum AggregatorRunnerError {
    #[error("Output builtin segment info not found in CairoPIE")]
    OutputBuiltinNoSegmentInfo,

    #[error("Unexpected relocatable value in output segment at offset {0}")]
    OutputSegmentUnexpectedRelocatable(usize),

    #[error("Output segment incomplete: expected {expected} values, found {found}")]
    IncompleteOutputSegment { expected: usize, found: usize },

    #[error("Aggregator execution failed: {0}")]
    AggregatorExecution(String),

    #[error("CairoPIE validity check failed: {0}")]
    PieValidation(String),

    #[error("Failed to create temp file for DA segment: {0}")]
    TempFileCreation(#[from] std::io::Error),

    #[error("Failed to read DA segment from temp file: {0}")]
    DaSegmentRead(String),

    #[error("No child CairoPIEs provided")]
    NoChildPies,

    #[error("Missing output builtin segment with index {0:?}")]
    MissingBuiltinSegment(BuiltinName),
}
