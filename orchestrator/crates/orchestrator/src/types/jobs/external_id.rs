use color_eyre::eyre::eyre;
use serde::{Deserialize, Serialize};

/// An external id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum ExternalId {
    /// A string.
    String(Box<str>),
    /// A number.
    Number(usize),
}

impl From<String> for ExternalId {
    #[inline]
    fn from(value: String) -> Self {
        ExternalId::String(value.into_boxed_str())
    }
}

impl From<usize> for ExternalId {
    #[inline]
    fn from(value: usize) -> Self {
        ExternalId::Number(value)
    }
}

impl ExternalId {
    /// Unwraps the external id as a string.
    ///
    /// # Panics
    ///
    /// This function panics if the provided external id not a string.
    #[track_caller]
    #[inline]
    pub fn unwrap_string(&self) -> color_eyre::Result<&str> {
        match self {
            ExternalId::String(s) => Ok(s),
            _ => Err(unwrap_external_id_failed("string", self)),
        }
    }

    /// Unwraps the external id as a number.
    ///
    /// # Panics
    ///
    /// This function panics if the provided external id is not a number.
    #[track_caller]
    #[inline]
    #[allow(dead_code)] // temporarily unused (until the other pull request uses it)
    pub fn unwrap_number(&self) -> color_eyre::Result<usize> {
        match self {
            ExternalId::Number(n) => Ok(*n),
            _ => Err(unwrap_external_id_failed("number", self)),
        }
    }
}


/// Returns an error indicating that the provided external id coulnd't be unwrapped.
fn unwrap_external_id_failed(expected: &str, got: &ExternalId) -> color_eyre::eyre::Error {
    eyre!("wrong ExternalId type: expected {}, got {:?}", expected, got)
}