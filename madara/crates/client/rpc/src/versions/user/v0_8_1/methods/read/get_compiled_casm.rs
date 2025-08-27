use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use anyhow::Context;
use starknet_types_core::felt::Felt;
use std::str::FromStr;

pub fn get_compiled_casm(starknet: &Starknet, class_hash: Felt) -> StarknetRpcResult<serde_json::Value> {
    let view = starknet.backend.view_on_latest();
    let class = view.get_class_info_and_compiled(&class_hash)?.ok_or(StarknetRpcApiError::class_hash_not_found())?;

    let sierra = class.as_sierra().ok_or(StarknetRpcApiError::unsupported_contract_class_version())?;

    // Using `Value::from_str` to deserialize `compiled_class` from a JSON string stored in the database.
    // Since `compiled_class` is stored as a raw JSON string in the DB, we need to parse it into a
    // `serde_json::Value` to work with it as a structured JSON object for serialization.
    let res =
        serde_json::Value::from_str(sierra.compiled.0.as_str()).context("Error serializing compiled contract class")?;

    Ok(res)
}
