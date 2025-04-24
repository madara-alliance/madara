use thiserror::Error;

/// TODO: move this code to a common crate
#[derive(Error, Debug, PartialEq)]
pub enum TypeError {
    #[error("Invalid metadata variant map: {0}")]
    InvalidMetadataVariantMap(String),
}
