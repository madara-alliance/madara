use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use httpmock::MockServer;
use orchestrator_atlantic_service::{AtlanticProverService, AtlanticQueryStatus, AtlanticValidatedArgs};
use orchestrator_prover_client_interface::{ProverClient, Task};
use orchestrator_utils::env_utils::get_env_var_or_panic;
use url::Url;

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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
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

    let task_result = atlantic_service.submit_task(Task::CairoPie(Box::new(cairo_pie)), None).await;

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
        atlantic_network: get_env_var_or_panic("MADARA_ORCHESTRATOR_ATLANTIC_NETWORK"),
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic);

    let atlantic_query_id = "01JPMKV7WFP4JTC0TTQSEAM9GW";
    let task_result = atlantic_service.atlantic_client.get_job_status(atlantic_query_id).await;
    assert!(task_result.is_ok());
}

#[tokio::test]
async fn atlantic_client_submit_task_and_get_job_status_with_mock_fact_hash() {
    let _ = env_logger::try_init();
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");

    println!("Submitting task to Atlantic");

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
    };

    // Create the Atlantic service with actual configuration
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic);

    // Load the Cairo PIE from the test data
    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    print!("Loading Cairo PIE from {}", cairo_pie_path);
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("Failed to read Cairo PIE zip file");

    // Submit the task to the actual Atlantic service
    let task_result = atlantic_service
        .submit_task(Task::CairoPie(Box::new(cairo_pie)), Some(14000000))
        .await
        .expect("Failed to submit task to Atlantic service");

    println!("Task submitted successfully. Query ID: {}", task_result);

    // Poll for job status until it's done or timeout is reached
    let max_retries = 30; // Set a reasonable number of retries
    let retry_delay = std::time::Duration::from_secs(10);
    let mut current_retry = 0;

    loop {
        if current_retry >= max_retries {
            panic!("Maximum retries reached. Test timed out.");
        }

        // Wait before checking status again
        tokio::time::sleep(retry_delay).await;
        current_retry += 1;

        println!("Checking job status (attempt {}/{})", current_retry, max_retries);

        // Get the current status of the job
        let status_result =
            atlantic_service.atlantic_client.get_job_status(&task_result).await.expect("Failed to get job status");

        println!("Current job status: {:?}", status_result.atlantic_query.status);

        match status_result.atlantic_query.status {
            AtlanticQueryStatus::Done => {
                // Job completed successfully
                println!("Job completed successfully!");

                // Check the fact hash if available
                if let Some(fact_hash) = status_result.atlantic_query.integrity_fact_hash {
                    println!("Integrity fact hash: {}", fact_hash);
                }

                if let Some(is_mocked) = status_result.atlantic_query.is_fact_mocked {
                    println!("Is fact mocked: {}", is_mocked);
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
                // Job is still processing, continue waiting
                log::info!("Job is still processing, waiting for next check...");
                continue;
            }
        }
    }

    // If we got here, the test passed
    assert!(true, "The test completed successfully");
}
