use starknet_types_core::felt::Felt;

use crate::contracts::ExecutionError;
use crate::core::storage::function_selector;

pub(crate) const FUNCTION_NAMES: &[&str] =
    &["get_asset_kind", "get_base_token_asset", "get_asset_min_size_increment", "get_name", "get_version"];

const GET_ASSET_KIND_INDEX: usize = 0;
const GET_BASE_TOKEN_ASSET_INDEX: usize = 1;
const GET_ASSET_MIN_SIZE_INCREMENT_INDEX: usize = 2;
const GET_NAME_INDEX: usize = 3;
const GET_VERSION_INDEX: usize = 4;

pub(super) fn get_asset_kind_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_ASSET_KIND_INDEX])
}

pub(super) fn get_base_token_asset_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_BASE_TOKEN_ASSET_INDEX])
}

pub(super) fn get_asset_min_size_increment_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_ASSET_MIN_SIZE_INCREMENT_INDEX])
}

pub(super) fn get_name_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_NAME_INDEX])
}

pub(super) fn get_version_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_VERSION_INDEX])
}

pub(super) fn get_function_name(selector: Felt) -> Option<&'static str> {
    if selector == get_asset_kind_selector() {
        Some(FUNCTION_NAMES[GET_ASSET_KIND_INDEX])
    } else if selector == get_base_token_asset_selector() {
        Some(FUNCTION_NAMES[GET_BASE_TOKEN_ASSET_INDEX])
    } else if selector == get_asset_min_size_increment_selector() {
        Some(FUNCTION_NAMES[GET_ASSET_MIN_SIZE_INCREMENT_INDEX])
    } else if selector == get_name_selector() {
        Some(FUNCTION_NAMES[GET_NAME_INDEX])
    } else if selector == get_version_selector() {
        Some(FUNCTION_NAMES[GET_VERSION_INDEX])
    } else {
        None
    }
}

pub(super) fn take_felt(input: &[Felt]) -> Result<Felt, ExecutionError> {
    if input.is_empty() {
        return Err(ExecutionError::ExecutionFailed("calldata underflow".to_string()));
    }
    Ok(input[0])
}
