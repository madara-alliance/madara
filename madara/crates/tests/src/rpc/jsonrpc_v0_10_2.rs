#[cfg(test)]
mod test_rpc_jsonrpc_v0_10_2 {
    use crate::{MadaraCmd, MadaraCmdBuilder};
    use rstest::rstest;
    use serde_json::{json, Map, Value};
    use tokio::sync::OnceCell;

    static MADARA_INSTANCE: OnceCell<MadaraCmd> = OnceCell::const_new();
    const ETH_TOKEN_ADDRESS: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
    const ETH_NAME_STORAGE_KEY: &str = "0x0341c1bdfd89f69748aa00b5742b03adbffd79b8e80cab5c50d91cd8c2a79be1";

    async fn get_madara() -> &'static MadaraCmd {
        MADARA_INSTANCE
            .get_or_init(|| async {
                let mut madara = MadaraCmdBuilder::new()
                    .args(["--full", "--network", "sepolia", "--sync-stop-at", "19", "--no-l1-sync"])
                    .run();

                madara.wait_for_ready().await;
                madara.wait_for_sync_to(19).await;
                madara
            })
            .await
    }

    async fn rpc_response(madara: &MadaraCmd, method: &str, params: Value) -> Value {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        let rpc_url = format!("{}/rpc/v0_10_2/", madara.rpc_url().trim_end_matches('/'));
        let response = reqwest::Client::new().post(rpc_url).json(&payload).send().await.expect("RPC request failed");
        assert!(response.status().is_success(), "RPC returned HTTP {}", response.status());
        let response: Value = response.json().await.expect("RPC response was not JSON");
        response
    }

    async fn rpc_result(madara: &MadaraCmd, method: &str, params: Value) -> Value {
        let response = rpc_response(madara, method, params).await;
        if let Some(error) = response.get("error") {
            panic!("RPC returned error: {error}");
        }
        response.get("result").cloned().expect("RPC response missing result")
    }

    fn assert_optional_proof_facts(value: &Value) {
        if let Some(proof_facts) = value.get("proof_facts") {
            assert!(proof_facts.is_array(), "proof_facts should be an array when present, got: {proof_facts:?}");
        }
    }

    fn event_filter_params(address: Option<Value>, continuation_token: Option<&str>, chunk_size: u64) -> Value {
        let mut filter = Map::new();
        filter.insert("from_block".to_string(), json!({"block_number": 0}));
        filter.insert("to_block".to_string(), json!({"block_number": 19}));
        filter.insert("keys".to_string(), json!([[]]));
        filter.insert("chunk_size".to_string(), Value::from(chunk_size));

        if let Some(address) = address {
            filter.insert("address".to_string(), address);
        }
        if let Some(token) = continuation_token {
            filter.insert("continuation_token".to_string(), Value::String(token.to_owned()));
        }

        Value::Object(Map::from_iter([("filter".to_string(), Value::Object(filter))]))
    }

    fn block_event_filter_params(block_number: u64, continuation_token: Option<&str>, chunk_size: u64) -> Value {
        let mut filter = Map::new();
        filter.insert("from_block".to_string(), json!({"block_number": block_number}));
        filter.insert("to_block".to_string(), json!({"block_number": block_number}));
        filter.insert("keys".to_string(), json!([[]]));
        filter.insert("chunk_size".to_string(), Value::from(chunk_size));

        if let Some(token) = continuation_token {
            filter.insert("continuation_token".to_string(), Value::String(token.to_owned()));
        }

        Value::Object(Map::from_iter([("filter".to_string(), Value::Object(filter))]))
    }

    fn expected_events_first_page() -> Value {
        json!({
            "events": [
                {
                    "from_address": ETH_TOKEN_ADDRESS,
                    "keys": ["0x3774b0545aabb37c45c1eddc6a7dae57de498aae6d5e3589e362d4b4323a533"],
                    "data": [
                        "0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8",
                        "0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"
                    ],
                    "block_hash": "0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c",
                    "block_number": 0,
                    "transaction_hash": "0x1bec64a9f5ff52154b560fd489ae2aabbfcb31062f7ea70c3c674ddf14b0add",
                    "event_index": 0,
                    "transaction_index": 5
                },
                {
                    "from_address": ETH_TOKEN_ADDRESS,
                    "keys": ["0x4595132f9b33b7077ebf2e7f3eb746a8e0a6d5c337c71cd8f9bf46cac3cfd7"],
                    "data": ["0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"],
                    "block_hash": "0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c",
                    "block_number": 0,
                    "transaction_hash": "0x1bec64a9f5ff52154b560fd489ae2aabbfcb31062f7ea70c3c674ddf14b0add",
                    "event_index": 1,
                    "transaction_index": 5
                }
            ],
            "continuation_token": "4-0"
        })
    }

    fn expected_events_second_page() -> Value {
        json!({
            "events": [
                {
                    "from_address": ETH_TOKEN_ADDRESS,
                    "keys": ["0x2e8a4ec40a36a027111fafdb6a46746ff1b0125d5067fbaebd8b5f227185a1e"],
                    "data": [
                        "0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5",
                        "0x0",
                        "0x4",
                        "0x4574686572",
                        "0x455448",
                        "0x12",
                        "0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f",
                        "0x0"
                    ],
                    "block_hash": "0x445152a69e628774b0f78a952e6f9ba0ffcda1374724b314140928fd2f31f4c",
                    "block_number": 4,
                    "transaction_hash": "0x3c9dfcd3fe66be18b661ee4ebb62520bb4f13d4182b040b3c2be9a12dbcc09b",
                    "event_index": 0,
                    "transaction_index": 0
                },
                {
                    "from_address": ETH_TOKEN_ADDRESS,
                    "keys": ["0x1205ec81562fc65c367136bd2fe1c0fff2d1986f70e4ba365e5dd747bd08753"],
                    "data": [
                        "0x23be95f90bf41685e18a4356e57b0cfdc1da22bf382ead8b64108353915c1e5",
                        "0x0",
                        "0x4",
                        "0x4574686572",
                        "0x455448",
                        "0x12",
                        "0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"
                    ],
                    "block_hash": "0x445152a69e628774b0f78a952e6f9ba0ffcda1374724b314140928fd2f31f4c",
                    "block_number": 4,
                    "transaction_hash": "0x24ae8900d238a120e927d6a5f2e4ddf85419e97020d92c1fadc615bff666ab1",
                    "event_index": 0,
                    "transaction_index": 1
                }
            ],
            "continuation_token": "6-0"
        })
    }

    fn expected_first_page_token(filtered_events: &[Value], page_size: usize) -> Option<String> {
        if filtered_events.len() <= page_size {
            return None;
        }

        let next_block_number =
            filtered_events[page_size].get("block_number").and_then(Value::as_u64).expect("missing next block number");
        let already_consumed_in_next_block = filtered_events[..page_size]
            .iter()
            .rev()
            .take_while(|event| event.get("block_number").and_then(Value::as_u64) == Some(next_block_number))
            .count();

        Some(format!("{next_block_number}-{already_consumed_in_next_block}"))
    }

    fn select_multi_address_regression_case(unfiltered_events: &[Value]) -> Option<(Vec<String>, Vec<Value>)> {
        let mut unique_addresses = Vec::new();
        for event in unfiltered_events {
            let address = event.get("from_address")?.as_str()?.to_owned();
            if !unique_addresses.contains(&address) {
                unique_addresses.push(address);
            }
        }

        for i in 0..unique_addresses.len() {
            for j in (i + 1)..unique_addresses.len() {
                let addresses = vec![unique_addresses[i].clone(), unique_addresses[j].clone()];
                let filtered_events = unfiltered_events
                    .iter()
                    .filter(|event| {
                        event
                            .get("from_address")
                            .and_then(Value::as_str)
                            .is_some_and(|from_address| addresses.iter().any(|address| address == from_address))
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                if filtered_events.len() < 3 {
                    continue;
                }

                if expected_first_page_token(&filtered_events, 2).is_some() {
                    return Some((addresses, filtered_events));
                }
            }
        }

        None
    }

    #[tokio::test]
    async fn test_raw_block_hash_and_number_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_blockHashAndNumber", json!({})).await;

        assert_eq!(result["block_number"], 19);
        assert_eq!(
            result["block_hash"].as_str(),
            Some("0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5")
        );
    }

    #[tokio::test]
    async fn test_raw_spec_version_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_specVersion", json!({})).await;

        assert_eq!(result.as_str(), Some("0.10.2"));
    }

    #[tokio::test]
    async fn test_raw_get_storage_at_with_last_update_block_v0_10_2() {
        let madara = get_madara().await;
        let params = json!({
            "contract_address": ETH_TOKEN_ADDRESS,
            "key": ETH_NAME_STORAGE_KEY,
            "block_id": {"block_number": 12},
            "response_flags": ["INCLUDE_LAST_UPDATE_BLOCK"]
        });

        let result = rpc_result(madara, "starknet_getStorageAt", params).await;
        assert_eq!(result["value"].as_str(), Some("0x4574686572"));
        assert!(
            result.get("last_update_block").and_then(|value| value.as_u64()).is_some(),
            "expected last_update_block in object response: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_raw_get_storage_at_without_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let params = json!({
            "contract_address": ETH_TOKEN_ADDRESS,
            "key": ETH_NAME_STORAGE_KEY,
            "block_id": {"block_number": 12}
        });

        let result = rpc_result(madara, "starknet_getStorageAt", params).await;
        assert_eq!(result.as_str(), Some("0x4574686572"));
    }

    #[tokio::test]
    async fn test_raw_get_state_update_contract_filter_v0_10_2() {
        let madara = get_madara().await;
        let address = ETH_TOKEN_ADDRESS;
        let result = rpc_result(
            madara,
            "starknet_getStateUpdate",
            json!({
                "block_id": {"block_number": 13},
                "contract_addresses": [address]
            }),
        )
        .await;

        let state_diff = result.get("state_diff").expect("missing state_diff");
        let storage_diffs = state_diff
            .get("storage_diffs")
            .and_then(|value| value.as_array())
            .expect("storage_diffs should be an array");
        assert_eq!(storage_diffs.len(), 1, "filtered response should keep one storage diff");
        assert_eq!(storage_diffs[0]["address"].as_str(), Some(address));

        let deployed_contracts = state_diff
            .get("deployed_contracts")
            .and_then(|value| value.as_array())
            .expect("deployed_contracts should be an array");
        assert!(deployed_contracts.is_empty(), "unexpected deployed_contracts after filter");
    }

    #[tokio::test]
    async fn test_raw_get_state_update_empty_filter_matches_unfiltered_v0_10_2() {
        let madara = get_madara().await;
        let unfiltered = rpc_result(madara, "starknet_getStateUpdate", json!({"block_id": {"block_number": 13}})).await;
        let empty_filter = rpc_result(
            madara,
            "starknet_getStateUpdate",
            json!({
                "block_id": {"block_number": 13},
                "contract_addresses": []
            }),
        )
        .await;

        assert_eq!(empty_filter, unfiltered);
    }

    #[tokio::test]
    async fn test_raw_trace_transaction_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_traceTransaction",
            json!({"transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a"}),
        )
        .await;

        let error = response.get("error").expect("expected trace to return error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(41));
        assert_eq!(error.get("message").and_then(|value| value.as_str()), Some("Transaction execution error"));
        let execution_error = error
            .get("data")
            .and_then(|value| value.get("execution_error"))
            .and_then(|value| value.as_str())
            .expect("missing execution_error");
        assert!(execution_error.contains("Unsupported protocol version"));
    }

    #[tokio::test]
    async fn test_raw_get_transaction_by_hash_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let tx_hash = "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a";

        let result = rpc_result(madara, "starknet_getTransactionByHash", json!({"transaction_hash": tx_hash})).await;
        assert_optional_proof_facts(&result);

        let result = rpc_result(
            madara,
            "starknet_getTransactionByHash",
            json!({"transaction_hash": tx_hash, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;
        assert_optional_proof_facts(&result);
    }

    #[tokio::test]
    async fn test_raw_get_transaction_by_hash_invalid_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_getTransactionByHash",
            json!({"transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a", "response_flags": ["UNKNOWN_FLAG"]}),
        )
        .await;

        let error = response.get("error").expect("expected response_flags error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(-32602));
    }

    #[tokio::test]
    async fn test_raw_get_transaction_by_block_id_and_index_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getTransactionByBlockIdAndIndex",
            json!({"block_id": {"block_number": 19}, "index": 0, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;

        assert_optional_proof_facts(&result);
    }

    #[tokio::test]
    async fn test_raw_get_block_with_txs_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getBlockWithTxs",
            json!({"block_id": {"block_number": 19}, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;

        let transactions =
            result.get("transactions").and_then(|value| value.as_array()).expect("transactions should be an array");

        for tx in transactions {
            assert_optional_proof_facts(tx);
        }
    }

    #[tokio::test]
    async fn test_raw_get_block_with_txs_without_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_getBlockWithTxs", json!({"block_id": {"block_number": 19}})).await;

        let transactions =
            result.get("transactions").and_then(|value| value.as_array()).expect("transactions should be an array");
        assert!(!transactions.is_empty(), "expected transactions without response_flags");
    }

    #[tokio::test]
    async fn test_raw_get_block_with_receipts_response_flags_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getBlockWithReceipts",
            json!({"block_id": {"block_number": 19}, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;

        let transactions =
            result.get("transactions").and_then(|value| value.as_array()).expect("transactions should be an array");

        for item in transactions {
            let tx = item.get("transaction").expect("transaction should exist in receipt item");
            assert_optional_proof_facts(tx);
        }
    }

    #[tokio::test]
    async fn test_raw_trace_block_transactions_return_initial_reads_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_traceBlockTransactions",
            json!({"block_id": {"block_number": 19}, "trace_flags": ["RETURN_INITIAL_READS"]}),
        )
        .await;

        let error = response.get("error").expect("expected trace to return error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(41));
        assert_eq!(error.get("message").and_then(|value| value.as_str()), Some("Transaction execution error"));
        let execution_error = error
            .get("data")
            .and_then(|value| value.get("execution_error"))
            .and_then(|value| value.as_str())
            .expect("missing execution_error");
        assert!(execution_error.contains("Unsupported protocol version"));
    }

    #[tokio::test]
    async fn test_raw_trace_block_transactions_invalid_trace_flags_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_traceBlockTransactions",
            json!({"block_id": {"block_number": 19}, "trace_flags": ["UNKNOWN_FLAG"]}),
        )
        .await;

        let error = response.get("error").expect("expected trace_flags error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(-32602));
    }

    #[tokio::test]
    async fn test_raw_simulate_transactions_return_initial_reads_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_simulateTransactions",
            json!({
                "block_id": {"block_number": 19},
                "transactions": [],
                "simulation_flags": ["RETURN_INITIAL_READS"]
            }),
        )
        .await;

        let error = response.get("error").expect("expected simulate to return error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(41));
        assert_eq!(error.get("message").and_then(|value| value.as_str()), Some("Transaction execution error"));
        let execution_error = error
            .get("data")
            .and_then(|value| value.get("execution_error"))
            .and_then(|value| value.as_str())
            .expect("missing execution_error");
        assert!(execution_error.contains("Unsupported protocol version"));
    }

    #[tokio::test]
    async fn test_raw_simulate_transactions_invalid_simulation_flags_v0_10_2() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_simulateTransactions",
            json!({
                "block_id": {"block_number": 19},
                "transactions": [],
                "simulation_flags": ["UNKNOWN_FLAG"]
            }),
        )
        .await;

        let error = response.get("error").expect("expected simulation_flags error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(-32602));
    }

    #[tokio::test]
    async fn test_raw_get_events_address_array_v0_10_2() {
        let madara = get_madara().await;
        let result =
            rpc_result(madara, "starknet_getEvents", event_filter_params(Some(json!([ETH_TOKEN_ADDRESS])), None, 2))
                .await;

        assert_eq!(result, expected_events_first_page());
    }

    #[tokio::test]
    async fn test_raw_get_events_empty_address_array_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_getEvents", event_filter_params(Some(json!([])), None, 2)).await;
        let unfiltered = rpc_result(madara, "starknet_getEvents", event_filter_params(None, None, 2)).await;

        assert_eq!(result, unfiltered);
    }

    #[tokio::test]
    async fn test_raw_get_events_single_address_array_v0_10_2() {
        let madara = get_madara().await;
        let array_result =
            rpc_result(madara, "starknet_getEvents", event_filter_params(Some(json!([ETH_TOKEN_ADDRESS])), None, 2))
                .await;
        let string_result =
            rpc_result(madara, "starknet_getEvents", event_filter_params(Some(json!(ETH_TOKEN_ADDRESS)), None, 2))
                .await;

        assert_eq!(array_result, string_result);
    }

    #[tokio::test]
    async fn test_raw_get_events_single_address_string_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getEvents",
            event_filter_params(Some(json!(ETH_TOKEN_ADDRESS)), Some("0-2"), 2),
        )
        .await;

        assert_eq!(result, expected_events_second_page());
    }

    #[rstest]
    #[tokio::test]
    async fn test_raw_get_events_multi_address_pagination_uses_filtered_offsets_v0_10_2() {
        let madara = get_madara().await;
        let unfiltered = rpc_result(madara, "starknet_getEvents", event_filter_params(None, None, 100)).await;
        let unfiltered_events = unfiltered
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .expect("unfiltered getEvents response should contain an events array");

        let (addresses, filtered_events) = select_multi_address_regression_case(&unfiltered_events)
            .expect("sample chain should contain a multi-address pagination regression case");
        let expected_token =
            expected_first_page_token(&filtered_events, 2).expect("regression case should span more than one page");
        let expected_first_page_events = filtered_events[..2].to_vec();

        let first_page =
            rpc_result(madara, "starknet_getEvents", event_filter_params(Some(json!(addresses)), None, 2)).await;
        assert_eq!(
            first_page.get("events").and_then(Value::as_array),
            Some(&expected_first_page_events),
            "first page should match the first filtered events",
        );
        assert_eq!(first_page.get("continuation_token").and_then(Value::as_str), Some(expected_token.as_str()));

        let second_page = rpc_result(
            madara,
            "starknet_getEvents",
            event_filter_params(Some(json!(addresses)), Some(expected_token.as_str()), 2),
        )
        .await;
        assert_eq!(
            second_page.get("events").and_then(Value::as_array).and_then(|events| events.first()),
            Some(&filtered_events[2]),
            "the next page should resume from the first excluded matching event",
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_raw_get_events_same_block_page_does_not_duplicate_last_event_v0_10_2() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_getEvents", block_event_filter_params(4, None, 20)).await;

        assert_eq!(
            result.get("events").and_then(Value::as_array),
            expected_events_second_page().get("events").and_then(Value::as_array),
            "same-block pagination should return each matching event once",
        );
        assert!(
            result.get("continuation_token").is_none() || result.get("continuation_token").is_some_and(Value::is_null),
            "single-block result should not advertise another page: {result:?}"
        );
    }
}
