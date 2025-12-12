use crate::constants::{CAIRO_PIE_PATH, MAX_RETRIES, RETRY_DELAY};
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use httpmock::MockServer;
use orchestrator_atlantic_service::types::{AtlanticCairoVm, AtlanticQueryStep};
use orchestrator_atlantic_service::{AtlanticProverService, AtlanticQueryStatus, AtlanticValidatedArgs};
use orchestrator_prover_client_interface::{CreateJobInfo, ProverClient, Task};
use orchestrator_utils::env_utils::get_env_var_or_panic;
use url::Url;
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
        atlantic_cairo_vm: AtlanticCairoVm::Rust,
        atlantic_result: AtlanticQueryStep::ProofGeneration,
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
        AtlanticProverService::with_test_params(mock_server.port(), &atlantic_params, &LayoutName::dynamic)
            .expect("Failed to create Atlantic service");

    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("failed to read cairo pie zip");

    // We don't need to send the steps because it's a mock fact hash.
    let task_result = atlantic_service
        .submit_task(Task::CreateJob(CreateJobInfo {
            cairo_pie: Box::new(cairo_pie),
            bucket_id: None,
            bucket_job_index: None,
            num_steps: None,
            external_id: uuid::Uuid::new_v4().to_string(),
        }))
        .await;

    assert!(task_result.is_ok());
    submit_mock.assert();
}

#[tokio::test]
async fn atlantic_client_does_not_resubmit_when_job_exists() {
    let _ = env_logger::try_init();
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    let external_id = uuid::Uuid::new_v4().to_string();
    let bucket_id = "bucket-123".to_string();
    let bucket_job_index = 1u64;

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
        atlantic_cairo_vm: AtlanticCairoVm::Rust,
        atlantic_result: AtlanticQueryStep::ProofGeneration,
    };

    let mock_server = MockServer::start();

    let search_mock = mock_server.mock(|when, then| {
        when.method("GET")
            .path("/atlantic-queries")
            .query_param("search", external_id.as_str())
            .query_param("limit", "1")
            .query_param("network", "TESTNET");

        then.status(200).header("content-type", "application/json").json_body(serde_json::json!({
            "atlanticQueries": [{
                "id": "existing_query_id",
                "externalId": external_id.clone(),
                "transactionId": null,
                "status": "RECEIVED",
                "step": null,
                "programHash": null,
                "integrityFactHash": null,
                "sharpFactHash": null,
                "layout": null,
                "isFactMocked": null,
                "chain": null,
                "jobSize": null,
                "declaredJobSize": null,
                "cairoVm": null,
                "cairoVersion": null,
                "steps": [],
                "errorReason": null,
                "submittedByClient": "client",
                "projectId": "project",
                "createdAt": "2024-01-01T00:00:00Z",
                "completedAt": null,
                "result": null,
                "network": "TESTNET",
                "hints": null,
                "sharpProver": null,
                "bucketId": bucket_id.clone(),
                "bucketJobIndex": bucket_job_index,
                "customerName": null,
                "isJobSizeValid": true,
                "isProofMocked": null,
                "client": {
                    "clientId": null,
                    "name": null,
                    "email": null,
                    "isEmailVerified": null,
                    "image": null
                }
            }],
            "total": 1
        }));
    });

    let submit_mock = mock_server.mock(|when, then| {
        when.method("POST").path("/atlantic-query");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({ "atlanticQueryId": "should_not_be_called" }));
    });

    let atlantic_service =
        AtlanticProverService::with_test_params(mock_server.port(), &atlantic_params, &LayoutName::dynamic)
            .expect("Failed to create Atlantic service");

    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("failed to read cairo pie zip");

    let task_result = atlantic_service
        .submit_task(Task::CreateJob(CreateJobInfo {
            cairo_pie: Box::new(cairo_pie),
            bucket_id: Some(bucket_id.clone()),
            bucket_job_index: Some(bucket_job_index),
            num_steps: None,
            external_id: external_id.clone(),
        }))
        .await
        .expect("submit_task should return existing job id");

    assert_eq!(task_result, "existing_query_id");
    search_mock.assert_calls(1);
    submit_mock.assert_calls(0);
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
        atlantic_cairo_vm: AtlanticCairoVm::Rust,
        atlantic_result: AtlanticQueryStep::ProofGeneration,
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic, None, None)
        .expect("Failed to create Atlantic service");

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
        cairo_verifier_program_hash: None,
        atlantic_cairo_vm: AtlanticCairoVm::Python,
        atlantic_result: AtlanticQueryStep::ProofGeneration,
    };
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic, None, None)
        .expect("Failed to create Atlantic service");

    let bucket_id = "01K0RN2JFJW3382CZPHRBC48NR";
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
        atlantic_cairo_vm: AtlanticCairoVm::Rust,
        atlantic_result: AtlanticQueryStep::ProofGeneration,
    };

    // Create the Atlantic service with actual configuration
    let atlantic_service = AtlanticProverService::new_with_args(&atlantic_params, &LayoutName::dynamic, None, None)
        .expect("Failed to create Atlantic service");

    // Load the Cairo PIE from the test data
    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).expect("Failed to read Cairo PIE zip file");

    // Submit the task to the actual Atlantic service
    let task_result = atlantic_service
        // We don't need to send the steps because it's a mock fact hash.
        .submit_task(Task::CreateJob(CreateJobInfo {
            cairo_pie: Box::new(cairo_pie),
            bucket_id: None,
            bucket_job_index: None,
            num_steps: None,
            external_id: uuid::Uuid::new_v4().to_string(),
        }))
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
