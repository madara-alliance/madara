use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum TypeError {
    #[error("Invalid metadata variant map: {0}")]
    InvalidMetadataVariantMap(String),
}
