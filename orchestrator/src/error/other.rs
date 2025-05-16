use color_eyre::eyre::eyre;
use std::fmt;

/// Wrapper Type for Other(<>) job type
///
/// Provides a generic error type for cases that don't fit into specific error categories
/// while maintaining error chain context.
#[derive(Debug)]
pub struct OtherError(pub color_eyre::eyre::Error);

impl fmt::Display for OtherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for OtherError {}

impl PartialEq for OtherError {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl From<color_eyre::eyre::Error> for OtherError {
    fn from(err: color_eyre::eyre::Error) -> Self {
        OtherError(err)
    }
}

impl From<String> for OtherError {
    fn from(error_string: String) -> Self {
        OtherError(eyre!(error_string))
    }
}
