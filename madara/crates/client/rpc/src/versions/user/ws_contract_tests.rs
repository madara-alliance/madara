use std::collections::BTreeSet;

use crate::test_utils::rpc_test_setup;
use crate::versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server;
use crate::versions::user::v0_8_1::StarknetWsRpcApiV0_8_1Server;
use crate::versions::user::v0_9_0::StarknetWsRpcApiV0_9_0Server;

fn ws_method_names<Context>(module: jsonrpsee::RpcModule<Context>) -> BTreeSet<String> {
    module.method_names().map(str::to_owned).collect()
}

#[test]
fn v0_8_1_ws_surface_contains_expected_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));

    assert!(methods.contains("starknet_V0_8_1_subscribeNewHeads"));
    assert!(methods.contains("starknet_V0_8_1_subscribeEvents"));
    assert!(methods.contains("starknet_V0_8_1_subscribeTransactionStatus"));
    assert!(methods.contains("starknet_V0_8_1_subscribePendingTransactions"));
    assert!(methods.contains("starknet_V0_8_1_unsubscribe"));
}

#[test]
fn v0_9_0_ws_surface_uses_new_transaction_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));

    assert!(methods.contains("starknet_V0_9_0_subscribeNewHeads"));
    assert!(methods.contains("starknet_V0_9_0_subscribeEvents"));
    assert!(methods.contains("starknet_V0_9_0_subscribeTransactionStatus"));
    assert!(methods.contains("starknet_V0_9_0_subscribeNewTransactions"));
    assert!(methods.contains("starknet_V0_9_0_subscribeNewTransactionReceipts"));
    assert!(methods.contains("starknet_V0_9_0_unsubscribe"));
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
