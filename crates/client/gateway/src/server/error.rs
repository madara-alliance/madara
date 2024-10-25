use std::fmt::{self, Display};

use hyper::Response;
use mc_db::MadaraStorageError;

use crate::error::StarknetError;

use super::helpers::internal_error_response;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error(transparent)]
    StarknetError(#[from] StarknetError),
    #[error("Internal server error: {0}")]
    InternalServerError(String),
}

impl From<MadaraStorageError> for GatewayError {
    fn from(e: MadaraStorageError) -> Self {
        log::error!(target: "gateway_errors", "Storage error: {}", e);
        Self::InternalServerError(e.to_string())
    }
}

impl From<GatewayError> for Response<String> {
    fn from(e: GatewayError) -> Response<String> {
        match e {
            GatewayError::StarknetError(e) => e.into(),
            GatewayError::InternalServerError(msg) => internal_error_response(&msg),
        }
    }
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError>;
}

impl<T, E: Display> ResultExt<T, E> for Result<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                log::error!(target: "gateway_errors", "{context}: {err:#}");
                Err(GatewayError::InternalServerError(err.to_string()))
            }
        }
    }
}

pub trait OptionExt<T> {
    fn ok_or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError> {
        match self {
            Some(val) => Ok(val),
            None => {
                log::error!(target: "gateway_errors", "{context}");
                Err(GatewayError::InternalServerError(context.to_string()))
            }
        }
    }
}
