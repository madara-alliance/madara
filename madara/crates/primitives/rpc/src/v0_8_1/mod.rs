//! v0.8.1 of the API.
pub use crate::custom::{BlockId, SyncingStatus};

mod starknet_api_openrpc;
mod starknet_ws_api;

pub use self::starknet_api_openrpc::*;
pub use self::starknet_ws_api::*;

// TODO: complete with all missing types of v0.8.1
pub use crate::v0_7_1::EmittedEvent;
