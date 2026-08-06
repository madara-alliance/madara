use std::collections::BTreeSet;

use crate::versions::user::v0_10_0::StarknetWsRpcApiV0_10_0Server;
use crate::versions::user::v0_10_2::StarknetWsRpcApiV0_10_2Server;
use crate::{rpc_api_user, test_utils::rpc_test_setup};

const LEGACY_WS_METHODS: &[&str] = &[
    "starknet_V0_8_1_subscribeNewHeads",
    "starknet_V0_8_1_subscribeEvents",
    "starknet_V0_8_1_subscribeTransactionStatus",
    "starknet_V0_8_1_subscribePendingTransactions",
    "starknet_V0_8_1_unsubscribe",
    "starknet_V0_9_0_subscribeNewHeads",
    "starknet_V0_9_0_subscribeEvents",
    "starknet_V0_9_0_subscribeTransactionStatus",
    "starknet_V0_9_0_subscribeNewTransactions",
    "starknet_V0_9_0_subscribeNewTransactionReceipts",
    "starknet_V0_9_0_subscribePendingTransactions",
    "starknet_V0_9_0_unsubscribe",
];

fn ws_method_names<Context>(module: jsonrpsee::RpcModule<Context>) -> BTreeSet<String> {
    module.method_names().map(str::to_owned).collect()
}

#[test]
fn merged_rpc_does_not_expose_legacy_ws_methods() {
    let (_, starknet) = rpc_test_setup();
    let methods = ws_method_names(rpc_api_user(&starknet).expect("Building user RPC module"));

    for method in LEGACY_WS_METHODS {
        assert!(!methods.contains(*method));
    }
}

#[tokio::test]
async fn legacy_ws_methods_return_method_not_found() {
    let (_, starknet) = rpc_test_setup();
    let module = rpc_api_user(&starknet).expect("Building user RPC module");

    for method in LEGACY_WS_METHODS {
        let request = format!(r#"{{"jsonrpc":"2.0","method":"{method}","id":1}}"#);
        let (response, _) = module.raw_json_request(&request, 1).await.expect("Legacy WS method request");
        let response: serde_json::Value = serde_json::from_str(&response).expect("Parsing JSON-RPC response");

        assert_eq!(response["error"]["code"], -32601, "{method} should return method not found");
    }
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
