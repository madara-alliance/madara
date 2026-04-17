use cairo_vm::vm::runners::cairo_pie::CairoPie;
use constants::CAIRO_PIE_PATH;
use httpmock::MockServer;
use orchestrator_prover_client_interface::{CreateJobInfo, ProverClient, TaskType};
use orchestrator_prover_client_interface::{Task, TaskStatus};
use orchestrator_sharp_service::types::CairoJobStatus;
use orchestrator_sharp_service::{SharpProverService, SharpValidatedArgs};
use orchestrator_utils::env_utils::get_env_var_or_panic;
use orchestrator_utils::test_utils::pem_from_env;
use rstest::rstest;
use serde_json::json;
use url::Url;

mod constants;

#[rstest]
#[tokio::test]
async fn prover_client_submit_task_works() {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    let sharp_params = SharpValidatedArgs {
        sharp_customer_id: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_CUSTOMER_ID"),
        sharp_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_URL")).unwrap(),
        sharp_user_crt: pem_from_env("MADARA_ORCHESTRATOR_SHARP_USER_CRT"),
        sharp_user_key: pem_from_env("MADARA_ORCHESTRATOR_SHARP_USER_KEY"),
        sharp_rpc_node_url: Url::parse(&get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_RPC_NODE_URL")).unwrap(),
        sharp_server_crt: pem_from_env("MADARA_ORCHESTRATOR_SHARP_SERVER_CRT"),
        gps_verifier_contract_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS"),
        sharp_settlement_layer: get_env_var_or_panic("MADARA_ORCHESTRATOR_SHARP_SETTLEMENT_LAYER"),
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

    assert!(sharp_service
        .submit_task(Task::CreateJob(CreateJobInfo {
            cairo_pie: Box::new(cairo_pie),
            bucket_id: None,
            bucket_job_index: None,
            num_steps: None,
            dedup_id: uuid::Uuid::new_v4().to_string(),
        }))
        .await
        .is_ok());

    sharp_add_job_call.assert();
}

// =============================================================================
// Unit tests for the pure status-mapping function (no SHARP client needed)
// =============================================================================

use orchestrator_sharp_service::map_sharp_status;
use orchestrator_sharp_service::types::{InvalidReason, SharpGetStatusResponse};

fn status_response(status: CairoJobStatus) -> SharpGetStatusResponse {
    SharpGetStatusResponse { status, invalid_reason: None, error_log: None, validation_done: None }
}

// --- Child job (TaskType::Job) mapping ---

#[rstest]
#[case::failed(CairoJobStatus::Failed, TaskStatus::Failed(String::new()))]
#[case::invalid(CairoJobStatus::Invalid, TaskStatus::Failed("Job is invalid: Unknown".to_string()))]
#[case::unknown(CairoJobStatus::Unknown, TaskStatus::Failed("Job not found: test-key".to_string()))]
#[case::processed(CairoJobStatus::Processed, TaskStatus::Succeeded)]
#[case::not_created(CairoJobStatus::NotCreated, TaskStatus::Processing)]
#[case::in_progress(CairoJobStatus::InProgress, TaskStatus::Processing)]
fn test_map_sharp_status_child_job(#[case] cairo_status: CairoJobStatus, #[case] expected: TaskStatus) {
    let res = status_response(cairo_status);
    assert_eq!(map_sharp_status(&TaskType::Job, "test-key", &res, None), expected);
}

#[rstest]
#[case::in_progress_validation_done(
    SharpGetStatusResponse { status: CairoJobStatus::InProgress, validation_done: Some(true), ..Default::default() },
    TaskStatus::Succeeded
)]
#[case::failed_with_error_log(
    SharpGetStatusResponse { status: CairoJobStatus::Failed, error_log: Some("pie decoding failed".to_string()), ..Default::default() },
    TaskStatus::Failed("pie decoding failed".to_string())
)]
#[case::invalid_with_reason(
    SharpGetStatusResponse { status: CairoJobStatus::Invalid, invalid_reason: Some(InvalidReason::InvalidCairoPieFileFormat), ..Default::default() },
    TaskStatus::Failed("Job is invalid: InvalidCairoPieFileFormat".to_string())
)]
fn test_child_job_edge_cases(#[case] res: SharpGetStatusResponse, #[case] expected: TaskStatus) {
    assert_eq!(map_sharp_status(&TaskType::Job, "test-key", &res, None), expected);
}

// --- Aggregation (TaskType::Aggregation) mapping ---

#[rstest]
#[case::failed(CairoJobStatus::Failed, None, TaskStatus::Failed(String::new()))]
#[case::invalid(CairoJobStatus::Invalid, None, TaskStatus::Failed("Applicative job is invalid: Unknown".to_string()))]
#[case::unknown(CairoJobStatus::Unknown, None, TaskStatus::Failed("Applicative job not found: agg-key".to_string()))]
#[case::in_progress(CairoJobStatus::InProgress, None, TaskStatus::Processing)]
#[case::not_created(CairoJobStatus::NotCreated, None, TaskStatus::Processing)]
fn test_map_sharp_status_aggregation(
    #[case] cairo_status: CairoJobStatus,
    #[case] fact_verified: Option<bool>,
    #[case] expected: TaskStatus,
) {
    let res = status_response(cairo_status);
    assert_eq!(map_sharp_status(&TaskType::Aggregation, "agg-key", &res, fact_verified), expected);
}

#[rstest]
#[case::fact_verified(Some(true), TaskStatus::Succeeded)]
#[case::fact_not_yet_on_chain(Some(false), TaskStatus::Processing)]
#[case::no_fact_provided(None, TaskStatus::Succeeded)]
fn test_aggregation_processed_with_fact_check(#[case] fact_verified: Option<bool>, #[case] expected: TaskStatus) {
    let res = status_response(CairoJobStatus::Processed);
    assert_eq!(map_sharp_status(&TaskType::Aggregation, "agg-key", &res, fact_verified), expected);
}
