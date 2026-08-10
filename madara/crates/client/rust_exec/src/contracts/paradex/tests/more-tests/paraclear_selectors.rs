use starknet_types_core::felt::Felt;

use crate::contracts::paradex::paraclear;
use crate::core::storage::function_selector;

#[test]
fn test_paraclear_supports_selector_settle_trade_v3() {
    let selector = function_selector("settle_trade_v3");
    assert!(paraclear::supports_selector(selector));
}

#[test]
fn test_paraclear_supports_selector_unknown() {
    let selector = Felt::from(999u64);
    assert!(!paraclear::supports_selector(selector));
}

#[test]
fn test_paraclear_get_function_name_settle_trade_v3() {
    let selector = function_selector("settle_trade_v3");
    assert_eq!(paraclear::get_function_name(selector), Some("settle_trade_v3".to_string()));
}

#[test]
fn test_paraclear_get_function_name_unknown() {
    let selector = Felt::from(1234u64);
    assert_eq!(paraclear::get_function_name(selector), None);
}
