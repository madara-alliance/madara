use std::str::FromStr;

use mp_block::{BlockId, BlockTag};
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_compiled_casm(starknet: &Starknet, class_hash: Felt) -> StarknetRpcResult<serde_json::Value> {
    let compiled_class_hash = starknet
        .backend
        .get_class_info(&BlockId::Tag(BlockTag::Latest), &class_hash)
        .or_internal_server_error("Error getting contract class info")?
        .ok_or(StarknetRpcApiError::class_hash_not_found())?
        .compiled_class_hash()
        .ok_or(StarknetRpcApiError::class_hash_not_found())?;

    let compiled_class = starknet
        .backend
        .get_sierra_compiled(&BlockId::Tag(BlockTag::Latest), &compiled_class_hash)
        .or_internal_server_error("Error getting compiled contract class")?
        .ok_or(StarknetRpcApiError::class_hash_not_found())?;

    // Using `Value::from_str` to deserialize `compiled_class` from a JSON string stored in the database.
    // Since `compiled_class` is stored as a raw JSON string in the DB, we need to parse it into a
    // `serde_json::Value` to work with it as a structured JSON object for serialization.
    let res = serde_json::Value::from_str(compiled_class.0.as_str())
        .or_internal_server_error("Error serializing compiled contract class")?;

    Ok(res)
}
