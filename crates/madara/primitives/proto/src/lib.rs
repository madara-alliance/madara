use mc_db::stream::{BlockStreamConfig, Direction};
use std::borrow::Cow;

#[allow(clippy::all)]
pub mod model {
    pub use crate::model_primitives::*;
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

mod classes;
mod events;
mod headers;
mod model_primitives;
mod transactions;

#[derive(thiserror::Error, Debug)]
pub enum FromModelError {
    #[error("Missing field: {0}")]
    MissingField(Cow<'static, str>),
    #[error("Invalid field: {0}")]
    InvalidField(Cow<'static, str>),
    #[error("Invalid enum variant for {ty}: {value}")]
    InvalidEnumVariant { ty: Cow<'static, str>, value: i32 },
    #[error("Legacy class conversion json error: {0:#}")]
    LegacyClassJsonError(serde_json::Error),
    #[error("Legacy class base64 decode error: {0:#}")]
    LegacyClassBase64Decode(base64::DecodeError),
}

impl FromModelError {
    pub fn missing_field(s: impl Into<Cow<'static, str>>) -> Self {
        Self::MissingField(s.into())
    }
    pub fn invalid_field(s: impl Into<Cow<'static, str>>) -> Self {
        Self::InvalidField(s.into())
    }
    pub fn invalid_enum_variant(ty: impl Into<Cow<'static, str>>, value: i32) -> Self {
        Self::InvalidEnumVariant { ty: ty.into(), value }
    }
}

impl From<BlockStreamConfig> for model::Iteration {
    fn from(value: BlockStreamConfig) -> Self {
        Self {
            direction: match value.direction {
                Direction::Forward => model::iteration::Direction::Forward,
                Direction::Backward => model::iteration::Direction::Backward,
            }
            .into(),
            limit: value.limit.unwrap_or_default(),
            step: value.step.get(),
            start: Some(model::iteration::Start::BlockNumber(value.start)),
        }
    }
}
