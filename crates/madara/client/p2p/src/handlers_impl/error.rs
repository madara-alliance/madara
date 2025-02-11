#![allow(unused)]

use crate::sync_handlers;
use std::fmt;

#[macro_export]
macro_rules! bail_internal_server_error {
    ($msg:literal $(,)?) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::Internal(anyhow::anyhow!($msg)))
    };
    ($err:expr $(,)?) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::Internal(anyhow::anyhow!($err)))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::Internal(anyhow::anyhow!($err, $($arg)*)))
    };
}

#[macro_export]
macro_rules! bail_bad_request {
    ($msg:literal $(,)?) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::BadRequest(format!($msg)))
    };
    ($err:expr $(,)?) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::BadRequest(format!($err)))
    };
    ($fmt:expr, $($arg:tt)*) => {
        return ::core::result::Result::Err($crate::sync_handlers::Error::BadRequest(format!($err, $($arg)*)))
    };
}

pub trait ResultExt<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, sync_handlers::Error>;
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error>;
    fn or_bad_request<C: fmt::Display>(self, context: C) -> Result<T, sync_handlers::Error>;
    fn or_else_bad_request<C: fmt::Display, F: FnOnce() -> C>(self, context_fn: F) -> Result<T, sync_handlers::Error>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T, E> for Result<T, E> {
    fn or_internal_server_error<C: fmt::Display>(self, context: C) -> Result<T, sync_handlers::Error> {
        self.map_err(|err| sync_handlers::Error::Internal(anyhow::anyhow!("{}: {:#}", context, E::into(err))))
    }
    fn or_else_internal_server_error<C: fmt::Display, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error> {
        self.map_err(|err| sync_handlers::Error::Internal(anyhow::anyhow!("{}: {:#}", context_fn(), E::into(err))))
    }

    fn or_bad_request<C: fmt::Display>(self, context: C) -> Result<T, sync_handlers::Error> {
        self.map_err(|err| sync_handlers::Error::BadRequest(format!("{}: {:#}", context, E::into(err)).into()))
    }
    fn or_else_bad_request<C: fmt::Display, F: FnOnce() -> C>(self, context_fn: F) -> Result<T, sync_handlers::Error> {
        self.map_err(|err| sync_handlers::Error::BadRequest(format!("{}: {:#}", context_fn(), E::into(err)).into()))
    }
}

pub trait OptionExt<T> {
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, sync_handlers::Error>;
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error>;
    fn ok_or_bad_request<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, sync_handlers::Error>;
    fn ok_or_else_bad_request<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error>;
}

impl<T> OptionExt<T> for Option<T> {
    fn ok_or_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, sync_handlers::Error> {
        self.ok_or_else(|| sync_handlers::Error::Internal(anyhow::anyhow!("{}", context)))
    }
    fn ok_or_else_internal_server_error<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error> {
        self.ok_or_else(|| sync_handlers::Error::Internal(anyhow::anyhow!("{}", context_fn())))
    }

    fn ok_or_bad_request<C: fmt::Display + fmt::Debug + Send + Sync + 'static>(
        self,
        context: C,
    ) -> Result<T, sync_handlers::Error> {
        self.ok_or_else(|| sync_handlers::Error::BadRequest(format!("{}", context).into()))
    }
    fn ok_or_else_bad_request<C: fmt::Display + fmt::Debug + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        context_fn: F,
    ) -> Result<T, sync_handlers::Error> {
        self.ok_or_else(|| sync_handlers::Error::BadRequest(format!("{}", context_fn()).into()))
    }
}
