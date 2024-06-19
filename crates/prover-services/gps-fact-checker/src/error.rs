use cairo_vm::program_hash::ProgramHashError;

#[derive(Debug, thiserror::Error)]
pub enum FactCheckerError {
    #[error("Fact registry call failed: {0}")]
    FactRegistry(#[source] alloy::contract::Error),
    #[error("Failed to compute program hash: {0}")]
    ProgramHashCompute(#[from] ProgramHashError),
    #[error("There is no additional data for the output builtin in Cairo PIE")]
    OutputBuiltinNoAdditionalData,
    #[error("There is no segment info for the output builtin in Cairo PIE")]
    OutputBuiltinNoSegmentInfo,
    #[error("Tree structure length is not even")]
    TreeStructureLenOdd,
    #[error("Tree structure is empty")]
    TreeStructureEmpty,
    #[error("Tree structure is too large")]
    TreeStructureTooLarge,
    #[error("Tree structure contains invalid values")]
    TreeStructureInvalid,
    #[error("Output pages length is unexpected")]
    OutputPagesLenUnexpected,
    #[error("Output page {0} has invalid start {1} (expected 0 < x < {2})")]
    OutputPagesInvalidStart(usize, usize, usize),
    #[error("Output page {0} has expected start {1} (expected{2})")]
    OutputPagesUnexpectedStart(usize, usize, usize),
    #[error("Output page {0} has invalid size {1} (expected 0 < x < {2})")]
    OutputPagesInvalidSize(usize, usize, usize),
    #[error("Output page {0} has unexpected id (expected {1})")]
    OutputPagesUnexpectedId(usize, usize),
    #[error("Output pages cover only {0} out of {1} output elements")]
    OutputPagesUncoveredOutput(usize, usize),
    #[error("Output segment is not found in the memory")]
    OutputSegmentNotFound,
    #[error("Output segment does not fit into the memory")]
    OutputSegmentInvalidRange,
    #[error("Output segment contains inconsistent offset {0} (expected {1})")]
    OutputSegmentInconsistentOffset(usize, usize),
    #[error("Output segment contains unexpected relocatable at position {0}")]
    OutputSegmentUnexpectedRelocatable(usize),
    #[error("Tree structure: pages count {0} is in invalid range (expected <= {1})")]
    TreeStructurePagesCountOutOfRange(usize, usize),
    #[error("Tree structure: nodes count {0} is in invalid range (expected <= {1})")]
    TreeStructureNodesCountOutOfRange(usize, usize),
    #[error("Tree structure: node stack contains more than one node")]
    TreeStructureRootInvalid,
    #[error("Tree structure: {0} pages were not processed")]
    TreeStructurePagesNotProcessed(usize),
    #[error("Tree structure: end offset {0} does not match the output length {1}")]
    TreeStructureEndOffsetInvalid(usize, usize),
    #[error("Tree structure: root offset {0} does not match the output length {1}")]
    TreeStructureRootOffsetInvalid(usize, usize),
}
