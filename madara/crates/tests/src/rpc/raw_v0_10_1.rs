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

    async fn rpc_result(madara: &MadaraCmd, method: &str, params: Value) -> Value {
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
        if let Some(error) = response.get("error") {
            panic!("RPC returned error: {error}");
        }
        response
            .get("result")
            .cloned()
            .expect("RPC response missing result")
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
        let result = rpc_result(
            madara,
            "starknet_traceTransaction",
            json!({"transaction_hash": "0x68fa87ed202095170a2f551017bf646180f43f4687553dc45e61598349a9a8a"}),
        )
        .await;

        assert_eq!(result["type"].as_str(), Some("L1_HANDLER"));
    }
}
