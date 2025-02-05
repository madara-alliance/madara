use crate::{model, sync_handlers};
use error::{OptionExt, ResultExt};
use mc_db::{
    stream::{BlockStreamConfig, Direction},
    MadaraBackend,
};
use mp_block::BlockId;
use std::{borrow::Cow, num::NonZeroU64};

mod classes;
mod error;
mod events;
mod headers;
mod state_diffs;
mod transactions;

pub use classes::*;
pub use events::*;
pub use headers::*;
pub use state_diffs::*;
pub use transactions::*;

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

pub fn block_stream_config(
    db: &MadaraBackend,
    value: model::Iteration,
) -> Result<BlockStreamConfig, sync_handlers::Error> {
    let direction = match value.direction() {
        model::iteration::Direction::Forward => Direction::Forward,
        model::iteration::Direction::Backward => Direction::Backward,
    };

    let start = match (value.start, &direction) {
        (Some(model::iteration::Start::BlockNumber(n)), _) => n,
        (Some(model::iteration::Start::Header(hash)), _) => db
            .get_block_n(&BlockId::Hash(hash.into()))
            .or_internal_server_error("Getting block_n from hash")?
            .ok_or_bad_request("Block not found")?,
        (None, Direction::Forward) => 0,
        (None, Direction::Backward) => {
            db.get_latest_block_n().or_internal_server_error("Getting latest block_n")?.unwrap_or(0)
        }
    };
    Ok(BlockStreamConfig {
        direction,
        start,
        // in protobuf fields default to 0 - we should not return any error in these cases.
        step: value.step.try_into().unwrap_or(NonZeroU64::MIN),
        limit: if value.limit == 0 { None } else { Some(value.limit) },
    })
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
