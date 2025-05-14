use crate::error::other::OtherError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ProvingError {
    #[error("Cairo PIE path is not specified - prover job #{internal_id:?}")]
    CairoPIEWrongPath { internal_id: String },

    #[error("Not able to read the cairo PIE file from the zip file provided.")]
    CairoPIENotReadable(String),

    #[error("Not able to get the PIE file from AWS S3 bucket.")]
    CairoPIEFileFetchFailed(String),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}
