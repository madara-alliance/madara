use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use httpmock::MockServer;
use orchestrator_atlantic_service::{AtlanticProverService, AtlanticQueryStatus, AtlanticValidatedArgs};
use orchestrator_prover_client_interface::{ProverClient, Task};
use orchestrator_utils::env_utils::get_env_var_or_panic;
use url::Url;

use crate::constants::{CAIRO_PIE_PATH, MAX_RETRIES, RETRY_DELAY};
mod constants;

#[tokio::test]
async fn atlantic_client_submit_task_when_mock_works() {
    let _ = env_logger::try_init();
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");
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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
        cairo_verifier_program_hash: None,
    };
    // Start a mock server
    let mock_server = MockServer::start();

    // Create a mock for the submit endpoint
    let submit_mock = mock_server.mock(|when, then| {
        when.method("POST").path("/atlantic-query");
        then.status(200).header("content-type", "application/json").json_body(serde_json::json!({
            "atlanticQueryId": "mock_query_id_123"
        }));
    });

    // Configure the service to use mock server
    let atlantic_service =
        AtlanticProverService::with_test_params(mock_server.port(), &atlantic_params, &LayoutName::dynamic);

    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("failed to read cairo pie zip");

    // We don't need to send the steps because it's a mock fact hash.
    let task_result = atlantic_service.submit_task(Task::CreateJob(Box::new(cairo_pie), None, None, None)).await;

    assert!(task_result.is_ok());
    submit_mock.assert();
}

#[tokio::test]
async fn atlantic_client_get_task_status_works() {
    let _ = env_logger::try_init();
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");
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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
        cairo_verifier_program_hash: None,
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic);

    let atlantic_query_id = "01JPMKV7WFP4JTC0TTQSEAM9GW";
    let task_result = atlantic_service.atlantic_client.get_job_status(atlantic_query_id).await;
    assert!(task_result.is_ok());
}

#[tokio::test]
async fn atlantic_client_get_bucket_status_works() {
    let _ = env_logger::try_init();
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");
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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic);

    let bucket_id = "01JY1P1NFSJC6T2G14H3MQKP3P";
    let task_result = atlantic_service.atlantic_client.get_bucket(bucket_id).await;
    assert!(task_result.is_ok());
}

#[tokio::test]
async fn atlantic_client_submit_task_and_get_job_status_with_mock_fact_hash() {
    let _ = env_logger::try_init();
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    // Initialize Atlantic parameters from environment variables
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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
        cairo_verifier_program_hash: None,
    };

    // Create the Atlantic service with actual configuration
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic);

    // Load the Cairo PIE from the test data
    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("Failed to read Cairo PIE zip file");

    // Submit the task to the actual Atlantic service
    let task_result = atlantic_service
        // We don't need to send the steps because it's a mock fact hash.
        .submit_task(Task::CreateJob(Box::new(cairo_pie), None, None, None))
        .await
        .expect("Failed to submit task to Atlantic service");

    let mut current_retry = 0;

    loop {
        if current_retry >= MAX_RETRIES {
            panic!("Maximum retries reached. Test timed out.");
        }

        // Wait before checking status again
        tokio::time::sleep(RETRY_DELAY).await;
        current_retry += 1;

        // Get the current status of the job
        let status_result =
            atlantic_service.atlantic_client.get_job_status(&task_result).await.expect("Failed to get job status");

        match status_result.atlantic_query.status {
            AtlanticQueryStatus::Done => {
                if let Some(is_mocked) = status_result.atlantic_query.is_fact_mocked {
                    assert!(is_mocked, "Expected fact to be mocked but it wasn't");
                }

                break; // Exit the loop when the job is done
            }
            AtlanticQueryStatus::Failed => {
                // Job failed
                let error_reason =
                    status_result.atlantic_query.error_reason.unwrap_or_else(|| "Unknown error".to_string());
                panic!("Job failed with reason: {}", error_reason);
            }
            _ => {
                continue;
            }
        }
    }
}
