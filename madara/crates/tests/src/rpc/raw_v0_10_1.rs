#[cfg(test)]
mod test_rpc_raw_v0_10_1 {
    use crate::{MadaraCmd, MadaraCmdBuilder};
    use serde_json::{json, Value};
    use tokio::sync::OnceCell;

    static MADARA_INSTANCE: OnceCell<MadaraCmd> = OnceCell::const_new();

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
        let response = reqwest::Client::new()
            .post(madara.rpc_url())
            .json(&payload)
            .send()
            .await
            .expect("RPC request failed");
        assert!(
            response.status().is_success(),
            "RPC returned HTTP {}",
            response.status()
        );
        let response: Value = response.json().await.expect("RPC response was not JSON");
        response
    }

    async fn rpc_result(madara: &MadaraCmd, method: &str, params: Value) -> Value {
        let response = rpc_response(madara, method, params).await;
        if let Some(error) = response.get("error") {
            panic!("RPC returned error: {error}");
        }
        response
            .get("result")
            .cloned()
            .expect("RPC response missing result")
    }

    fn assert_optional_proof_facts(value: &Value) {
        if let Some(proof_facts) = value.get("proof_facts") {
            assert!(
                proof_facts.is_array(),
                "proof_facts should be an array when present, got: {proof_facts:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_raw_block_hash_and_number_v0_10_1() {
        let madara = get_madara().await;
        let result = rpc_result(madara, "starknet_blockHashAndNumber", json!({})).await;

        assert_eq!(result["block_number"], 19);
        assert_eq!(
            result["block_hash"].as_str(),
            Some("0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5")
        );
    }

    #[tokio::test]
    async fn test_raw_trace_transaction_v0_10_1() {
        let madara = get_madara().await;
        let response = rpc_response(
            madara,
            "starknet_traceTransaction",
            json!({"transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a"}),
        )
        .await;

        let error = response.get("error").expect("expected trace to return error");
        assert_eq!(error.get("code").and_then(|value| value.as_i64()), Some(41));
        assert_eq!(
            error.get("message").and_then(|value| value.as_str()),
            Some("Transaction execution error")
        );
        let execution_error = error
            .get("data")
            .and_then(|value| value.get("execution_error"))
            .and_then(|value| value.as_str())
            .expect("missing execution_error");
        assert!(execution_error.contains("Unsupported protocol version"));
    }

    #[tokio::test]
    async fn test_raw_get_transaction_by_hash_response_flags_v0_10_1() {
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
    async fn test_raw_get_transaction_by_block_id_and_index_response_flags_v0_10_1() {
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
    async fn test_raw_get_block_with_txs_response_flags_v0_10_1() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getBlockWithTxs",
            json!({"block_id": {"block_number": 19}, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;

        let transactions = result
            .get("transactions")
            .and_then(|value| value.as_array())
            .expect("transactions should be an array");

        for tx in transactions {
            assert_optional_proof_facts(tx);
        }
    }

    #[tokio::test]
    async fn test_raw_get_block_with_receipts_response_flags_v0_10_1() {
        let madara = get_madara().await;
        let result = rpc_result(
            madara,
            "starknet_getBlockWithReceipts",
            json!({"block_id": {"block_number": 19}, "response_flags": ["INCLUDE_PROOF_FACTS"]}),
        )
        .await;

        let transactions = result
            .get("transactions")
            .and_then(|value| value.as_array())
            .expect("transactions should be an array");

        for item in transactions {
            let tx = item.get("transaction").expect("transaction should exist in receipt item");
            assert_optional_proof_facts(tx);
        }
    }
}
