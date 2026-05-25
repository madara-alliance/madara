use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use starknet_types_core::felt::Felt;
use std::str::FromStr;

pub fn get_compiled_casm(starknet: &Starknet, class_hash: Felt) -> StarknetRpcResult<serde_json::Value> {
    let view = starknet.backend.view_on_latest();
    let class = view.get_class_info_and_compiled(&class_hash)?.ok_or(StarknetRpcApiError::class_hash_not_found())?;

    let sierra = class.as_sierra().ok_or(StarknetRpcApiError::class_hash_not_found())?;

    // `compiled_class` is stored as raw JSON in the DB, so parse it back into a structured JSON value.
    let res = serde_json::Value::from_str(sierra.compiled.0.as_str()).map_err(|error| {
        StarknetRpcApiError::CompilationFailed {
            error: format!("Error deserializing compiled contract class from database: {error:#}").into(),
        }
    })?;

    Ok(res)
}
