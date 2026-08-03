use std::collections::BTreeSet;

use crate::versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server;
use crate::versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server;
use crate::{rpc_api_user, test_utils::rpc_test_setup};

fn ws_method_names<Context>(module: jsonrpsee::RpcModule<Context>) -> BTreeSet<String> {
    module.method_names().map(str::to_owned).collect()
}

#[test]
fn merged_rpc_does_not_expose_legacy_ws_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(rpc_api_user(&starknet).expect("Building user RPC module"));

    assert!(!methods.contains("starknet_V0_8_1_subscribeNewHeads"));
    assert!(!methods.contains("starknet_V0_8_1_subscribeEvents"));
    assert!(!methods.contains("starknet_V0_8_1_subscribeTransactionStatus"));
    assert!(!methods.contains("starknet_V0_8_1_subscribePendingTransactions"));
    assert!(!methods.contains("starknet_V0_8_1_unsubscribe"));
    assert!(!methods.contains("starknet_V0_9_0_subscribeNewHeads"));
    assert!(!methods.contains("starknet_V0_9_0_subscribeEvents"));
    assert!(!methods.contains("starknet_V0_9_0_subscribeTransactionStatus"));
    assert!(!methods.contains("starknet_V0_9_0_subscribeNewTransactions"));
    assert!(!methods.contains("starknet_V0_9_0_subscribeNewTransactionReceipts"));
    assert!(!methods.contains("starknet_V0_9_0_unsubscribe"));
    assert!(!methods.contains("starknet_V0_9_0_subscribePendingTransactions"));
}

#[test]
fn v0_10_0_ws_surface_uses_new_transaction_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(StarknetWsRpcApiV0_10_0Server::into_rpc(starknet));

    assert!(methods.contains("starknet_V0_10_0_subscribeNewHeads"));
    assert!(methods.contains("starknet_V0_10_0_subscribeEvents"));
    assert!(methods.contains("starknet_V0_10_0_subscribeTransactionStatus"));
    assert!(methods.contains("starknet_V0_10_0_subscribeNewTransactions"));
    assert!(methods.contains("starknet_V0_10_0_subscribeNewTransactionReceipts"));
    assert!(methods.contains("starknet_V0_10_0_unsubscribe"));
    assert!(!methods.contains("starknet_V0_10_0_subscribePendingTransactions"));
}

#[test]
fn v0_10_2_ws_surface_matches_new_transaction_spec_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(StarknetWsRpcApiV0_10_2Server::into_rpc(starknet));

    assert!(methods.contains("starknet_V0_10_2_subscribeNewHeads"));
    assert!(methods.contains("starknet_V0_10_2_subscribeEvents"));
    assert!(methods.contains("starknet_V0_10_2_subscribeTransactionStatus"));
    assert!(methods.contains("starknet_V0_10_2_subscribeNewTransactions"));
    assert!(methods.contains("starknet_V0_10_2_subscribeNewTransactionReceipts"));
    assert!(methods.contains("starknet_V0_10_2_unsubscribe"));
    assert!(!methods.contains("starknet_V0_10_2_subscribePendingTransactions"));
}
