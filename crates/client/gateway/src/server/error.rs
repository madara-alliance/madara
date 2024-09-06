use std::fmt;

use hyper::{Body, Response};
use mc_db::MadaraStorageError;

use crate::error::StarknetError;

use super::helpers::internal_error_response;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error(transparent)]
    StarknetError(#[from] StarknetError),
    #[error("Internal server error")]
    InternalServerError,
}

impl From<MadaraStorageError> for GatewayError {
    fn from(e: MadaraStorageError) -> Self {
        log::error!("Storage error: {}", e);
        Self::InternalServerError
    }
}

impl From<GatewayError> for Response<Body> {
    fn from(e: GatewayError) -> Response<Body> {
        match e {
            GatewayError::StarknetError(e) => e.into(),
            GatewayError::InternalServerError => internal_error_response(),
        }
    }
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError>;
}

impl<T, E: Into<GatewayError>> ResultExt<T, E> for Result<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, GatewayError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                let err = format!("{}: {:#}", context, E::into(err));
                log::error!(target: "gateway_errors", "{:#}", err);
                Err(GatewayError::InternalServerError)
            }
        }
    }
}
