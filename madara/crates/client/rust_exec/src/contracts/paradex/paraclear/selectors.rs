use starknet_types_core::felt::Felt;

use crate::core::storage::function_selector;

pub(crate) const FUNCTION_NAMES: &[&str] = &["settle_trade_v3"];

pub(super) fn settle_trade_v3_selector() -> Felt {
    function_selector(FUNCTION_NAMES[0])
}

pub(super) fn get_function_name(selector: Felt) -> Option<&'static str> {
    if selector == settle_trade_v3_selector() {
        Some(FUNCTION_NAMES[0])
    } else {
        None
    }
}
