use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use constants::CAIRO_PIE_PATH;
use httpmock::MockServer;
// ProverClient
use orchestrator_prover_client_interface::{CreateJobInfo, ProverClient, TaskType};
use orchestrator_prover_client_interface::{Task, TaskStatus};
use orchestrator_sharp_service::types::CairoJobStatus;
use orchestrator_sharp_service::{SharpProverService, SharpValidatedArgs};
use orchestrator_utils::env_utils::get_env_var_or_panic;
use rstest::rstest;
use serde_json::json;
use url::Url;

use crate::constants::{TEST_FACT, TEST_JOB_ID};

mod constants;

#[rstest]
#[tokio::test]
async fn prover_client_submit_task_works() {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    let sharp_params = SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL")).unwrap(),
        sharp_user_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL")).unwrap(),
        sharp_server_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        sharp_proof_layout: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
        sharp_settlement_layer: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SETTLEMENT_LAYER"),
    };

    let server = MockServer::start();
    let sharp_service = SharpProverService::with_test_params(server.port(), &sharp_params, &LayoutName::dynamic);
    let cairo_pie_path = env!("CARGO_MANIFEST_DIR").to_string() + CAIRO_PIE_PATH;
    let cairo_pie = CairoPie::read_zip_file(cairo_pie_path.as_ref()).unwrap();

    let sharp_response = json!(
            {
                "code" : "JOB_RECEIVED_SUCCESSFULLY"
            }
    );
    let customer_id = get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID");
    let sharp_add_job_call = server.mock(|when, then| {
        when.path_includes("/add_job").query_param("customer_id", customer_id.as_str());
        then.status(200).body(serde_json::to_vec(&sharp_response).unwrap());
    });

    assert!(sharp_service
        .submit_task(Task::CreateJob(CreateJobInfo {
            cairo_pie: Box::new(cairo_pie),
            bucket_id: None,
            bucket_job_index: None,
            num_steps: None,
        }))
        .await
        .is_ok());

    sharp_add_job_call.assert();
}

#[rstest]
#[case(CairoJobStatus::Failed)]
#[case(CairoJobStatus::Invalid)]
#[case(CairoJobStatus::Unknown)]
#[case(CairoJobStatus::InProgress)]
#[case(CairoJobStatus::NotCreated)]
#[case(CairoJobStatus::Processed)]
#[ignore]
#[case(CairoJobStatus::Onchain)]
#[tokio::test]
async fn prover_client_get_task_status_works(#[case] cairo_job_status: CairoJobStatus) {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    let sharp_params = SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL")).unwrap(),
        sharp_user_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL")).unwrap(),
        sharp_server_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        sharp_proof_layout: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
        sharp_settlement_layer: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SETTLEMENT_LAYER"),
    };

    let server = MockServer::start();
    let sharp_service = SharpProverService::with_test_params(server.port(), &sharp_params, &LayoutName::dynamic);
    let customer_id = get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID");

    let sharp_add_job_call = server.mock(|when, then| {
        when.path_includes("/get_status").query_param("customer_id", customer_id.as_str());
        then.status(200).body(serde_json::to_vec(&get_task_status_sharp_response(&cairo_job_status)).unwrap());
    });

    let task_status =
        sharp_service.get_task_status(TaskType::Job, TEST_JOB_ID, Some(TEST_FACT.to_string()), false).await.unwrap();
    assert_eq!(task_status, get_task_status_expectation(&cairo_job_status), "Cairo Job Status assertion failed");

    sharp_add_job_call.assert();
}

fn get_task_status_expectation(cairo_job_status: &CairoJobStatus) -> TaskStatus {
    match cairo_job_status {
        CairoJobStatus::Failed => TaskStatus::Failed("Sharp task failed".to_string()),
        CairoJobStatus::Invalid => TaskStatus::Failed("Task is invalid: InvalidCairoPieFileFormat".to_string()),
        CairoJobStatus::Unknown => TaskStatus::Failed("".to_string()),
        CairoJobStatus::InProgress | CairoJobStatus::NotCreated | CairoJobStatus::Processed => TaskStatus::Processing,
        CairoJobStatus::Onchain => TaskStatus::Failed(format!("Fact {} is not valid or not registered", TEST_FACT)),
    }
}

fn get_task_status_sharp_response(cairo_job_status: &CairoJobStatus) -> serde_json::Value {
    match cairo_job_status {
        CairoJobStatus::Failed => json!(
            {
                "status" : "FAILED",
                "error_log" : "Sharp task failed"
            }
        ),
        CairoJobStatus::Invalid => json!(
            {
                "status": "INVALID",
                "invalid_reason": "INVALID_CAIRO_PIE_FILE_FORMAT",
                "error_log": "The Cairo PIE file has a wrong format. Deserialization ended with exception: Invalid prefix for zip file.."}
        ),
        CairoJobStatus::Unknown => json!(
            {
                "status" : "FAILED"
            }
        ),
        CairoJobStatus::InProgress => json!(
            {
                "status": "IN_PROGRESS",
                "validation_done": false
            }
        ),
        CairoJobStatus::NotCreated => json!(
            {
                "status": "NOT_CREATED",
                "validation_done": false
            }
        ),
        CairoJobStatus::Processed => json!(
            {
                "status": "PROCESSED",
                "validation_done": false
            }
        ),
        CairoJobStatus::Onchain => json!(
            {
                "status": "ONCHAIN",
                "validation_done": true
            }
        ),
    }
}
