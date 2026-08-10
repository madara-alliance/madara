use starknet_types_core::felt::Felt;

use crate::core::storage::function_selector;

pub(crate) const FUNCTION_NAMES: &[&str] = &[
    "get_value",
    "get_values_with_funding_indices",
    "get_funding_index",
    "set_prices_and_funding_snapshot",
    "get_latest_snapshot_id",
    "decimals",
    "get_name",
    "get_version",
];

const GET_VALUE_INDEX: usize = 0;
const GET_VALUES_WITH_FUNDING_INDICES_INDEX: usize = 1;
const GET_FUNDING_INDEX_INDEX: usize = 2;
const SET_PRICES_AND_FUNDING_SNAPSHOT_INDEX: usize = 3;
const GET_LATEST_SNAPSHOT_ID_INDEX: usize = 4;
const DECIMALS_INDEX: usize = 5;
const GET_NAME_INDEX: usize = 6;
const GET_VERSION_INDEX: usize = 7;

pub(super) fn get_value_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_VALUE_INDEX])
}

pub(super) fn get_values_with_funding_indices_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_VALUES_WITH_FUNDING_INDICES_INDEX])
}

pub(super) fn get_funding_index_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_FUNDING_INDEX_INDEX])
}

pub(super) fn set_prices_and_funding_snapshot_selector() -> Felt {
    function_selector(FUNCTION_NAMES[SET_PRICES_AND_FUNDING_SNAPSHOT_INDEX])
}

pub(super) fn get_latest_snapshot_id_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_LATEST_SNAPSHOT_ID_INDEX])
}

pub(super) fn decimals_selector() -> Felt {
    function_selector(FUNCTION_NAMES[DECIMALS_INDEX])
}

pub(super) fn get_name_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_NAME_INDEX])
}

pub(super) fn get_version_selector() -> Felt {
    function_selector(FUNCTION_NAMES[GET_VERSION_INDEX])
}

pub(super) fn get_function_name(selector: Felt) -> Option<&'static str> {
    if selector == get_value_selector() {
        Some(FUNCTION_NAMES[GET_VALUE_INDEX])
    } else if selector == get_values_with_funding_indices_selector() {
        Some(FUNCTION_NAMES[GET_VALUES_WITH_FUNDING_INDICES_INDEX])
    } else if selector == get_funding_index_selector() {
        Some(FUNCTION_NAMES[GET_FUNDING_INDEX_INDEX])
    } else if selector == set_prices_and_funding_snapshot_selector() {
        Some(FUNCTION_NAMES[SET_PRICES_AND_FUNDING_SNAPSHOT_INDEX])
    } else if selector == get_latest_snapshot_id_selector() {
        Some(FUNCTION_NAMES[GET_LATEST_SNAPSHOT_ID_INDEX])
    } else if selector == decimals_selector() {
        Some(FUNCTION_NAMES[DECIMALS_INDEX])
    } else if selector == get_name_selector() {
        Some(FUNCTION_NAMES[GET_NAME_INDEX])
    } else if selector == get_version_selector() {
        Some(FUNCTION_NAMES[GET_VERSION_INDEX])
    } else {
        None
    }
}
