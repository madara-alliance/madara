use std::fmt;

use crate::StarknetRpcApiError;

pub fn display_internal_server_error(err: impl fmt::Display) {
    tracing::error!(target: "rpc_errors", "{:#}", err);
}

#[macro_export]
macro_rules! bail_internal_server_error {
    ($msg:literal $(,)?) => {{
        $crate::utils::display_internal_server_error(anyhow::anyhow!($msg));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    }};
    ($err:expr $(,)?) => {
        $crate::utils::display_internal_server_error(anyhow::anyhow!($err));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::utils::display_internal_server_error(anyhow::anyhow!($fmt, $($arg)*));
        return ::core::result::Result::Err($crate::StarknetRpcApiError::InternalServerError.into())
    };
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError>;
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError>;
    fn or_contract_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T, E> for Result<T, E> {
    #[inline]
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context, E::into(err)));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    #[inline]
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                display_internal_server_error(format!("{}: {:#}", context_fn(), E::into(err)));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    // TODO: should this be a thing?
    #[inline]
    fn or_contract_error<C: fmt::Display>(self, context: C) -> Result<T, StarknetRpcApiError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                tracing::error!(target: "rpc_errors", "Contract storage error: {context}: {:#}", E::into(err));
                Err(StarknetRpcApiError::ContractError)
            }
        }
    }
}

pub trait OptionExt<T> {
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, StarknetRpcApiError>;
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError>;
}

impl<T> OptionExt<T> for Option<T> {
    #[inline]
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Some(val) => Ok(val),
            None => {
                display_internal_server_error(anyhow::Error::msg(context));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }

    #[inline]
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, StarknetRpcApiError> {
        match self {
            Some(val) => Ok(val),
            None => {
                display_internal_server_error(anyhow::Error::msg(context_fn()));
                Err(StarknetRpcApiError::InternalServerError)
            }
        }
    }
}
