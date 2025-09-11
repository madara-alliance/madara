/// Endpoint to fetch the artifacts from the herodotus service
pub(crate) const ATLANTIC_FETCH_ARTIFACTS_BASE_URL: &str = "https://storage.googleapis.com/hero-atlantic";

// File names of the artifacts that are downloaded from the herodotus service
pub const CAIRO_PIE_FILE_NAME: &str = "pie.cairo0.zip";
pub const SNOS_OUTPUT_FILE_NAME: &str = "snos_output.json";
pub const PROOF_FILE_NAME: &str = "proof.json";

/// Endpoint to download the proof from the herodotus service
pub(crate) const ATLANTIC_PROOF_URL: &str = "https://storage.googleapis.com/hero-atlantic/queries/{}/proof.json";

// Aggregator job configurations
pub(crate) const AGGREGATOR_USE_KZG_DA: bool = true;
pub(crate) const AGGREGATOR_FULL_OUTPUT: bool = false;
