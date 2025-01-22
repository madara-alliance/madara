use atlantic_service::{AtlanticProverService, AtlanticValidatedArgs};
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use httpmock::MockServer;
use prover_client_interface::{ProverClient, Task};
use url::Url;
use utils::env_utils::get_env_var_or_panic;

use crate::constants::CAIRO_PIE_PATH;

mod constants;

#[tokio::test]
async fn atlantic_client_submit_task_when_mock_works() {
    let _ = env_logger::try_init();
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");
    let atlantic_params = AtlanticValidatedArgs {
        atlantic_api_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_API_KEY"),
        atlantic_service_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL")).unwrap(),
        atlantic_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL")).unwrap(),
        atlantic_mock_fact_hash: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH"),
        atlantic_prover_type: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE"),
        atlantic_settlement_layer: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER"),
        atlantic_verifier_contract_address: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS",
        ),
    };
    // Start a mock server
    let mock_server = MockServer::start();

    // Create a mock for the submit endpoint
    let submit_mock = mock_server.mock(|when, then| {
        when.method("POST").path("/v1/l1/atlantic-query/proof-generation-verification");
        then.status(200).header("content-type", "application/json").json_body(serde_json::json!({
            "atlanticQueryId": "mock_query_id_123"
        }));
    });

    // Configure the service to use mock server
    let atlantic_service = AtlanticProverService::with_test_params(mock_server.port(), &atlantic_params);

    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("failed to read cairo pie zip");

    let task_result = atlantic_service.submit_task(Task::CairoPie(Box::new(cairo_pie)), LayoutName::dynamic).await;

    assert!(task_result.is_ok());
    submit_mock.assert();
}

#[tokio::test]
async fn atlantic_client_get_task_status_works() {
    let _ = env_logger::try_init();
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");
    let atlantic_params = AtlanticValidatedArgs {
        atlantic_api_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_API_KEY"),
        atlantic_service_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_SERVICE_URL")).unwrap(),
        atlantic_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL")).unwrap(),
        atlantic_mock_fact_hash: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_MOCK_FACT_HASH"),
        atlantic_prover_type: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_PROVER_TYPE"),
        atlantic_settlement_layer: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_SETTLEMENT_LAYER"),
        atlantic_verifier_contract_address: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS",
        ),
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params);

    let atlantic_query_id = "01JDY6EKVQD8QYR8HE64WZC9VB";
    let task_result = atlantic_service.atlantic_client.get_job_status(atlantic_query_id).await;
    assert!(task_result.is_ok());
}
