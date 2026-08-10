use starknet_types_core::felt::Felt;

use crate::contracts::paradex::paraclear;
use crate::contracts::paradex::schema::paraclear_types::OrderCategory;

use super::super::fixtures::{dynamic_fee_request, encode_trade_request_v3_for_test, sample_trade};

#[test]
fn test_decode_trade_request_v3_roundtrip() {
    let trade = sample_trade(OrderCategory::API, OrderCategory::Dynamic(dynamic_fee_request()));
    let calldata = encode_trade_request_v3_for_test(&trade);
    let (decoded, rest) = paraclear::decode_trade_request_v3(&calldata).expect("decode should succeed");
    assert!(rest.is_empty());
    assert_eq!(decoded, trade);
}

#[test]
fn test_decode_trade_request_v3_underflow() {
    let empty: Vec<Felt> = Vec::new();
    let err = paraclear::decode_trade_request_v3(&empty).unwrap_err();
    assert!(format!("{err}").contains("calldata underflow"));
}

#[test]
fn test_decode_order_v3_roundtrip() {
    let trade = sample_trade(OrderCategory::RPI, OrderCategory::Interactive);
    let calldata = encode_trade_request_v3_for_test(&trade);
    let (_, rest) = paraclear::decode_trade_request_v3(&calldata).expect("decode should succeed");
    assert!(rest.is_empty());

    let order_bytes = super::super::fixtures::encode_order_v3_for_test(&trade.maker_order);
    let (decoded, remaining) = paraclear::decode_order_v3_for_test(&order_bytes).expect("decode order");
    assert!(remaining.is_empty());
    assert_eq!(decoded, trade.maker_order);
}

#[test]
fn test_decode_order_v3_underflow() {
    let order_bytes = vec![Felt::from(1u64)];
    let err = paraclear::decode_order_v3_for_test(&order_bytes).unwrap_err();
    assert!(format!("{err}").contains("calldata underflow"));
}

#[test]
fn test_decode_order_category_unspecified() {
    let input = vec![Felt::from(0u64)];
    let (cat, rest) = paraclear::decode_order_category_for_test(&input).expect("decode category");
    assert!(rest.is_empty());
    assert!(matches!(cat, OrderCategory::Unspecified));
}

#[test]
fn test_decode_order_category_api() {
    let input = vec![Felt::from(1u64)];
    let (cat, rest) = paraclear::decode_order_category_for_test(&input).expect("decode category");
    assert!(rest.is_empty());
    assert!(matches!(cat, OrderCategory::API));
}

#[test]
fn test_decode_order_category_rpi() {
    let input = vec![Felt::from(2u64)];
    let (cat, rest) = paraclear::decode_order_category_for_test(&input).expect("decode category");
    assert!(rest.is_empty());
    assert!(matches!(cat, OrderCategory::RPI));
}

#[test]
fn test_decode_order_category_interactive() {
    let input = vec![Felt::from(3u64)];
    let (cat, rest) = paraclear::decode_order_category_for_test(&input).expect("decode category");
    assert!(rest.is_empty());
    assert!(matches!(cat, OrderCategory::Interactive));
}

#[test]
fn test_decode_order_category_dynamic() {
    let input = vec![Felt::from(4u64), Felt::from(7u64), Felt::from(8u64), Felt::from(9u64)];
    let (cat, rest) = paraclear::decode_order_category_for_test(&input).expect("decode category");
    assert!(rest.is_empty());
    match cat {
        OrderCategory::Dynamic(fee) => {
            assert_eq!(fee.fee, Felt::from(7u64));
            assert_eq!(fee.fee_cap, Felt::from(8u64));
            assert_eq!(fee.fee_floor, Felt::from(9u64));
        }
        _ => panic!("expected dynamic category"),
    }
}

#[test]
fn test_encode_trade_request_v3_matches_decode() {
    let trade = sample_trade(OrderCategory::Unspecified, OrderCategory::API);
    let calldata = encode_trade_request_v3_for_test(&trade);
    let (decoded, rest) = paraclear::decode_trade_request_v3(&calldata).expect("decode should succeed");
    assert!(rest.is_empty());
    assert_eq!(decoded, trade);
}

#[test]
fn test_extract_trade_markets_v3() {
    let trade = sample_trade(OrderCategory::API, OrderCategory::RPI);
    let calldata = encode_trade_request_v3_for_test(&trade);
    let markets = paraclear::extract_trade_markets_v3(&calldata).expect("extract markets");
    assert_eq!(markets, vec![trade.maker_order.market, trade.taker_order.market]);
}

#[test]
fn test_extract_trade_request_v3() {
    let trade = sample_trade(OrderCategory::API, OrderCategory::RPI);
    let calldata = encode_trade_request_v3_for_test(&trade);
    let extracted = paraclear::extract_trade_request_v3(&calldata).expect("extract trade");
    assert_eq!(extracted, trade);
}
