use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use constants::CAIRO_PIE_PATH;
use httpmock::MockServer;
// ProverClient
use prover_client_interface::ProverClient;
use prover_client_interface::{Task, TaskStatus};
use rstest::rstest;
use serde_json::json;
use sharp_service::{SharpProverService, SharpValidatedArgs};
use starknet_os::sharp::CairoJobStatus;
use url::Url;
use utils::env_utils::get_env_var_or_panic;

use crate::constants::TEST_FACT;

mod constants;

#[rstest]
#[tokio::test]
async fn prover_client_submit_task_works() {
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");

    let sharp_params = SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL")).unwrap(),
        sharp_user_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL")).unwrap(),
        sharp_server_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        sharp_proof_layout: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
    };

    let server = MockServer::start();
    let sharp_service = SharpProverService::with_test_params(server.port(), &sharp_params);
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

    let cairo_pie = Box::new(cairo_pie);
    assert!(sharp_service.submit_task(Task::CairoPie(cairo_pie), LayoutName::dynamic).await.is_ok());

    sharp_add_job_call.assert();
}

#[rstest]
#[case(CairoJobStatus::FAILED)]
#[case(CairoJobStatus::INVALID)]
#[case(CairoJobStatus::UNKNOWN)]
#[case(CairoJobStatus::IN_PROGRESS)]
#[case(CairoJobStatus::NOT_CREATED)]
#[case(CairoJobStatus::PROCESSED)]
#[ignore]
#[case(CairoJobStatus::ONCHAIN)]
#[tokio::test]
async fn prover_client_get_task_status_works(#[case] cairo_job_status: CairoJobStatus) {
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env file");

    let sharp_params = SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL")).unwrap(),
        sharp_user_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL")).unwrap(),
        sharp_server_crt: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        sharp_proof_layout: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_PROOF_LAYOUT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
    };

    let server = MockServer::start();
    let sharp_service = SharpProverService::with_test_params(server.port(), &sharp_params);
    let customer_id = get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID");

    let sharp_add_job_call = server.mock(|when, then| {
        when.path_includes("/get_status").query_param("customer_id", customer_id.as_str());
        then.status(200).body(serde_json::to_vec(&get_task_status_sharp_response(&cairo_job_status)).unwrap());
    });

    let task_status = sharp_service.get_task_status("c31381bf-4739-4667-b5b8-b08af1c6b1c7", TEST_FACT).await.unwrap();
    assert_eq!(task_status, get_task_status_expectation(&cairo_job_status), "Cairo Job Status assertion failed");

    sharp_add_job_call.assert();
}

fn get_task_status_expectation(cairo_job_status: &CairoJobStatus) -> TaskStatus {
    match cairo_job_status {
        CairoJobStatus::FAILED => TaskStatus::Failed("Sharp task failed".to_string()),
        CairoJobStatus::INVALID => TaskStatus::Failed("Task is invalid: INVALID_CAIRO_PIE_FILE_FORMAT".to_string()),
        CairoJobStatus::UNKNOWN => TaskStatus::Failed("".to_string()),
        CairoJobStatus::IN_PROGRESS | CairoJobStatus::NOT_CREATED | CairoJobStatus::PROCESSED => TaskStatus::Processing,
        CairoJobStatus::ONCHAIN => TaskStatus::Failed(format!("Fact {} is not valid or not registered", TEST_FACT)),
    }
}

fn get_task_status_sharp_response(cairo_job_status: &CairoJobStatus) -> serde_json::Value {
    match cairo_job_status {
        CairoJobStatus::FAILED => json!(
            {
                "status" : "FAILED",
                "error_log" : "Sharp task failed"
            }
        ),
        CairoJobStatus::INVALID => json!(
            {
                "status": "INVALID",
                "invalid_reason": "INVALID_CAIRO_PIE_FILE_FORMAT",
                "error_log": "The Cairo PIE file has a wrong format. Deserialization ended with exception: Invalid prefix for zip file.."}
        ),
        CairoJobStatus::UNKNOWN => json!(
            {
                "status" : "FAILED"
            }
        ),
        CairoJobStatus::IN_PROGRESS => json!(
            {
                "status": "IN_PROGRESS",
                "validation_done": false
            }
        ),
        CairoJobStatus::NOT_CREATED => json!(
            {
                "status": "NOT_CREATED",
                "validation_done": false
            }
        ),
        CairoJobStatus::PROCESSED => json!(
            {
                "status": "PROCESSED",
                "validation_done": false
            }
        ),
        CairoJobStatus::ONCHAIN => json!(
            {
                "status": "ONCHAIN",
                "validation_done": true
            }
        ),
    }
}
