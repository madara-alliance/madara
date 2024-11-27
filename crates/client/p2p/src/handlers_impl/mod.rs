mod error;
mod headers;

use crate::{model, sync_handlers};
use error::{OptionExt, ResultExt};
pub use headers::*;
use mc_db::{
    stream::{BlockStreamConfig, Direction},
    MadaraBackend,
};
use mp_block::BlockId;
use mp_convert::FeltExt;
use starknet_core::types::Felt;
use std::num::NonZeroU64;

impl TryFrom<model::Felt252> for Felt {
    type Error = sync_handlers::Error;
    fn try_from(value: model::Felt252) -> Result<Self, Self::Error> {
        Self::from_slice_be_checked(&value.elements).or_bad_request("Malformated felt")
    }
}
impl From<Felt> for model::Felt252 {
    fn from(value: Felt) -> Self {
        Self { elements: value.to_bytes_be().into() }
    }
}

impl TryFrom<model::Hash> for Felt {
    type Error = sync_handlers::Error;
    fn try_from(value: model::Hash) -> Result<Self, Self::Error> {
        Self::from_slice_be_checked(&value.elements).or_bad_request("Malformated felt")
    }
}
impl From<Felt> for model::Hash {
    fn from(value: Felt) -> Self {
        Self { elements: value.to_bytes_be().into() }
    }
}

impl From<Felt> for model::Address {
    fn from(value: Felt) -> Self {
        Self { elements: value.to_bytes_be().into() }
    }
}

impl From<u128> for model::Uint128 {
    fn from(value: u128) -> Self {
        let b = value.to_be_bytes();
        let low = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let high = u64::from_be_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        Self { low, high }
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
            .get_block_n(&BlockId::Hash(hash.try_into()?))
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
