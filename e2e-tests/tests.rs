extern crate e2e_tests;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::time::{Duration, Instant};

use chrono::{SubsecRound, Utc};
use e2e_tests::anvil::AnvilSetup;
use e2e_tests::mock_server::MockResponseBodyType;
use e2e_tests::sharp::SharpClient;
use e2e_tests::starknet_client::StarknetClient;
use e2e_tests::utils::{get_mongo_db_client, read_state_update_from_file, vec_u8_to_hex_string};
use e2e_tests::{MongoDbServer, Orchestrator};
use mongodb::bson::doc;
use orchestrator::core::client::database::constant::{BATCHES_COLLECTION, JOBS_COLLECTION};
use orchestrator::core::client::queue::sqs::InnerSQS;
use orchestrator::core::client::SQS;
use orchestrator::types::batch::Batch;
use orchestrator::types::constant::{
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, ON_CHAIN_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use orchestrator::types::jobs::external_id::ExternalId;
use orchestrator::types::jobs::job_item::JobItem;
use orchestrator::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata, SettlementContext,
    SettlementContextData, SnosMetadata, StateUpdateMetadata,
};
use orchestrator::types::jobs::types::{JobStatus, JobType};
use orchestrator::types::params::database::DatabaseArgs;
use orchestrator::types::params::QueueArgs;
use orchestrator::types::queue::QueueType;
use orchestrator::worker::parser::job_queue_message::JobQueueMessage;
use orchestrator_utils::env_utils::{get_env_var_optional_or_panic, get_env_var_or_panic};
use rstest::rstest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use starknet::core::types::{Felt, MaybePendingStateUpdate};
use uuid::Uuid;

/// Expected DB state struct
#[derive(PartialEq, Debug)]
struct ExpectedDBState {
    internal_id: String,
    job_type: JobType,
    job_status: JobStatus,
    version: i32,
}

#[allow(dead_code)]
/// Initial setup for e2e tests
struct Setup {
    mongo_db_instance: MongoDbServer,
    starknet_client: StarknetClient,
    sharp_client: SharpClient,
    env_vector: HashMap<String, String>,
}

const DUMMY_BUCKET_ID: &str = "81c54e74-b685-4dfc-bb8d-a0d4be7c79f8";

impl Setup {
    pub async fn new(l2_block_number: String) -> Self {
        let db_params = DatabaseArgs {
            connection_uri: get_env_var_or_panic("MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL"),
            database_name: get_env_var_or_panic("MADARA_ORCHESTRATOR_DATABASE_NAME"),
        };

        let mongo_db_instance = MongoDbServer::run(db_params);
        println!("✅ Mongo DB setup completed");

        let starknet_client = StarknetClient::new();
        println!("✅ Starknet/Madara client setup completed");

        let sharp_client = SharpClient::new();
        println!("✅ Sharp client setup completed");

        let anvil_setup = AnvilSetup::new();
        let (starknet_core_contract_address, verifier_contract_address) = anvil_setup.deploy_contracts().await;
        println!("✅ Anvil setup completed");

        let mut env_vec: HashMap<String, String> = HashMap::new();

        let env_vars = dotenvy::vars();
        for (key, value) in env_vars {
            env_vec.insert(key, value);
        }

        env_vec
            .insert("MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL".to_string(), mongo_db_instance.endpoint().to_string());

        // Adding other values to the environment variables vector
        env_vec.insert("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL".to_string(), anvil_setup.rpc_url.to_string());
        env_vec.insert("MADARA_ORCHESTRATOR_SHARP_URL".to_string(), sharp_client.url());

        // Adding impersonation for operator as our own address here.
        // As we are using test contracts thus we don't need any impersonation.
        // But that logic is being used in integration tests so to keep that. We
        // add this address here.
        // Anvil.addresses[0]
        env_vec.insert(
            "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS".to_string(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
        );
        env_vec.insert(
            "MADARA_ORCHESTRATOR_GPS_VERIFIER_CONTRACT_ADDRESS".to_string(),
            verifier_contract_address.to_string(),
        );
        env_vec.insert(
            "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS".to_string(),
            starknet_core_contract_address.to_string(),
        );
        env_vec.insert("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS".to_string(), l2_block_number.clone());
        env_vec.insert(
            "MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS".to_string(),
            (l2_block_number.parse::<u64>().unwrap() + 2).to_string(),
        );

        Self { mongo_db_instance, starknet_client, sharp_client, env_vector: env_vec }
    }

    pub fn mongo_db_instance(&self) -> &MongoDbServer {
        &self.mongo_db_instance
    }

    #[allow(dead_code)]
    pub fn starknet_client(&mut self) -> &mut StarknetClient {
        &mut self.starknet_client
    }

    pub fn sharp_client(&mut self) -> &mut SharpClient {
        &mut self.sharp_client
    }

    pub fn envs(&self) -> Vec<(String, String)> {
        self.env_vector.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

#[rstest]
#[case("309146".to_string())]
#[tokio::test]
async fn test_orchestrator_workflow(#[case] l2_block_number: String) {
    // Fetching the env vars from the test env file as these will be used in
    // setting up of the test and during orchestrator run too.

    use e2e_tests::node::OrchestratorMode;
    println!("Loading .env file");
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");

    // E2E tests assume that we are not passing ARNs
    let aws_prefix = get_env_var_optional_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX");
    let aws_identifier = get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_SQS_QUEUE_IDENTIFIER");

    let queue_params = match aws_prefix {
        Some(prefix) => QueueArgs {
            queue_template_identifier: orchestrator::types::params::AWSResourceIdentifier::Name(format!(
                "{}_{}",
                prefix, aws_identifier,
            )),
        },
        None => QueueArgs {
            queue_template_identifier: orchestrator::types::params::AWSResourceIdentifier::Name(aws_identifier),
        },
    };

    let mut setup_config = Setup::new(l2_block_number.clone()).await;
    // Setup Cloud
    // Setup orchestrator cloud
    Orchestrator::new(OrchestratorMode::Setup, setup_config.envs());
    println!("✅ Orchestrator cloud setup completed");

    // Put SNOS job data in DB
    let snos_job_id = put_job_data_in_db_snos(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    // Put Proving job data in DB
    put_job_data_in_db_proving(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    // Mocking SHARP client responses
    mock_add_job_endpoint_output(setup_config.sharp_client()).await;
    mock_close_bucket_endpoint_output(setup_config.sharp_client()).await;
    mock_create_bucket_endpoint_output(setup_config.sharp_client()).await;
    mock_get_agg_task_id_endpoint_output(setup_config.sharp_client()).await;

    // Add SNOS job in processing queue
    put_snos_job_in_processing_queue(snos_job_id, queue_params)
        .await
        .expect("Failed to put SNOS job in processing queue");

    println!("✅ Orchestrator setup completed.");

    // Run orchestrator
    let mut orchestrator =
        Orchestrator::new(OrchestratorMode::Run, setup_config.envs()).expect("Failed to start orchestrator");
    orchestrator.wait_till_started().await;

    println!("✅ Orchestrator started");

    // Adding State checks in DB for validation of tests

    // Check 1: Check that the batch has been created properly
    wait_for_batch_state(Duration::from_secs(1500), 1, setup_config.mongo_db_instance())
        .await
        .expect("❌ After Batching state DB state assertion failed.");
    println!("✅ Batching state DB state assertion passed");

    let expected_state_after_snos_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::SnosRun,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(1500),
        l2_block_number.clone(),
        setup_config.mongo_db_instance(),
        expected_state_after_snos_job,
    )
    .await;
    assert!(test_result.is_ok(), "❌ After Snos Job state DB state assertion failed.");
    println!("✅ Snos Job state DB state assertion passed");

    // Check 2: Check that the Proof Creation Job has been completed correctly
    let expected_state_after_proving_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::ProofCreation,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(1500),
        l2_block_number.clone(),
        setup_config.mongo_db_instance(),
        expected_state_after_proving_job,
    )
    .await;
    assert!(test_result.is_ok(), "❌ After Proving Job state DB state assertion failed.");
    println!("✅ ProofCreation Job state DB state assertion passed");

    // Check 3: Check that the Aggregator Job has been completed correctly
    let expected_state_after_agg_job = ExpectedDBState {
        internal_id: String::from("1"),
        job_type: JobType::Aggregator,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(1500),
        String::from("1"),
        setup_config.mongo_db_instance(),
        expected_state_after_agg_job,
    )
    .await;
    assert!(test_result.is_ok(), "❌ After Aggregator Job state DB state assertion failed.");
    println!("✅ Aggregator Job state DB state assertion passed");

    // Check 4: Check that the State Transition Job has been completed correctly
    let expected_state_after_da_job = ExpectedDBState {
        internal_id: String::from("1"),
        job_type: JobType::StateTransition,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(1500),
        String::from("1"),
        setup_config.mongo_db_instance(),
        expected_state_after_da_job,
    )
    .await;
    assert!(test_result.is_ok(), "❌ After Update State Job state DB state assertion failed.");
    println!("✅ UpdateState Job state DB state assertion passed");
}

/// Function to check db for expected state continuously
async fn wait_for_db_state(
    timeout: Duration,
    l2_block_for_testing: String,
    mongo_db_server: &MongoDbServer,
    expected_db_state: ExpectedDBState,
) -> Result<(), String> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        let db_state =
            get_job_state_by_type(mongo_db_server, l2_block_for_testing.clone(), expected_db_state.job_type.clone())
                .await
                .expect("Failed to get database state");

        match db_state {
            Some(db_state) => {
                if db_state == expected_db_state {
                    return Ok(());
                }
            }
            None => {
                println!("Expected state not found yet for {:?}. Waiting..", expected_db_state.job_type);
            }
        }
        tokio::time::sleep(Duration::from_millis(5000)).await;
    }

    Err(format!("Timed out waiting for expected state: {:?}", expected_db_state))
}

/// Function to check DB for expected batch state continuously
async fn wait_for_batch_state(timeout: Duration, index: u64, mongo_db_server: &MongoDbServer) -> Result<(), String> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        let batch = get_batch_by_index(mongo_db_server, index).await.expect("Failed to get batch state");

        match batch {
            Some(batch) => {
                if batch.index == index {
                    update_batch_state(mongo_db_server, index)
                        .await
                        .expect("Failed to update batch state to Completed");
                    return Ok(());
                }
            }
            None => {
                println!("Expected state not found yet for Batching. Waiting..");
            }
        }
        tokio::time::sleep(Duration::from_millis(5000)).await;
    }

    Err("Timed out waiting for expected state of batch".to_string())
}

/// Fetch the job from the database
async fn get_job_state_by_type(
    mongo_db_server: &MongoDbServer,
    l2_block_for_testing: String,
    job_type: JobType,
) -> color_eyre::Result<Option<ExpectedDBState>> {
    let mongo_db_client = get_mongo_db_client(mongo_db_server).await;
    let collection = mongo_db_client.database("orchestrator").collection::<JobItem>(JOBS_COLLECTION);
    let filter = doc! { "internal_id": l2_block_for_testing, "job_type" : mongodb::bson::to_bson(&job_type)? };
    let job = collection.find_one(filter, None).await?;
    match job {
        Some(job) => Ok(Some(ExpectedDBState {
            internal_id: job.internal_id,
            job_type: job.job_type,
            job_status: job.status,
            version: job.version,
        })),
        None => Ok(None),
    }
}

/// Get Batch from DB
async fn get_batch_by_index(mongo_db_server: &MongoDbServer, index: u64) -> color_eyre::Result<Option<Batch>> {
    let mongo_db_client = get_mongo_db_client(mongo_db_server).await;
    let collection = mongo_db_client.database("orchestrator").collection::<Batch>(BATCHES_COLLECTION);
    let filter = doc! { "index": index as i64 };
    let batch = collection.find_one(filter, None).await?;
    match batch {
        Some(batch) => Ok(Some(batch)),
        None => Ok(None),
    }
}

/// Update Batch state to Completed in DB
async fn update_batch_state(mongo_db_server: &MongoDbServer, index: u64) -> color_eyre::Result<()> {
    let mongo_db_client = get_mongo_db_client(mongo_db_server).await;
    let collection = mongo_db_client.database("orchestrator").collection::<Batch>(BATCHES_COLLECTION);
    let filter = doc! { "index": index as i64 };
    let update = doc! { "$set": { "is_batch_ready": true, "status": "Closed" } };
    collection.update_one(filter, update, None).await?;
    Ok(())
}

// --------------------------- Queue functions ---------------------------

/// Adding SNOS job in JOB_PROCESSING_QUEUE so that the job is triggered
/// as soon as it is picked up by orchestrator
pub async fn put_snos_job_in_processing_queue(id: Uuid, queue_params: QueueArgs) -> color_eyre::Result<()> {
    let message = JobQueueMessage { id };

    let config = aws_config::from_env().load().await;
    let queue = SQS::new(&config, &queue_params);
    let queue_name = InnerSQS::get_queue_name_from_type(
        &queue_params.queue_template_identifier.to_string(),
        &QueueType::SnosJobProcessing,
    );
    let queue_url = queue.inner.get_queue_url_from_client(queue_name.as_str()).await?;
    put_message_in_queue(message, queue_url).await?;
    Ok(())
}

pub async fn put_message_in_queue(message: JobQueueMessage, queue_url: String) -> color_eyre::Result<()> {
    let config = aws_config::from_env().load().await;
    let client = aws_sdk_sqs::Client::new(&config);

    let rsp = client.send_message().queue_url(queue_url).message_body(serde_json::to_string(&message)?).send().await?;

    println!("✅ Successfully sent message with ID: {:?}", rsp.message_id());

    Ok(())
}

// --------------------------- Mocks ---------------------------

/// Mocks the endpoint for sharp client
pub async fn mock_add_job_endpoint_output(sharp_client: &mut SharpClient) {
    // Add job response,
    let add_job_response = json!(
        {
            "code" : "JOB_RECEIVED_SUCCESSFULLY"
        }
    );
    sharp_client.add_mock_on_endpoint(
        "/add_job",
        vec!["".to_string()],
        Some(200),
        MockResponseBodyType::Json(add_job_response),
    );

    // Getting job response
    let get_job_response = json!(
        {
                "status": "ONCHAIN",
                "validation_done": true
        }
    );
    sharp_client.add_mock_on_endpoint(
        "/get_status",
        vec!["".to_string()],
        Some(200),
        MockResponseBodyType::Json(get_job_response),
    );
}

/// Mocks get aggregator task ID for sharp client
pub async fn mock_get_agg_task_id_endpoint_output(sharp_client: &mut SharpClient) {
    let get_agg_task_id_response = json!(
        {
            "task_id": DUMMY_BUCKET_ID
        }
    );
    sharp_client.add_mock_on_endpoint(
        "/aggregator_task_id",
        vec!["".to_string()],
        Some(200),
        MockResponseBodyType::Json(get_agg_task_id_response),
    )
}

/// Mocks create bucket endpoint for sharp client
pub async fn mock_create_bucket_endpoint_output(sharp_client: &mut SharpClient) {
    let create_bucket_response = json!(
        {
            "code" : "BUCKET_CREATED_SUCCESSFULLY",
            "bucket_id": DUMMY_BUCKET_ID,
        }
    );
    sharp_client.add_mock_on_endpoint(
        "/create_bucket",
        vec!["".to_string()],
        Some(200),
        MockResponseBodyType::Json(create_bucket_response),
    )
}

/// Mocks the endpoint for sharp client
pub async fn mock_close_bucket_endpoint_output(sharp_client: &mut SharpClient) {
    let close_bucket_response = json!(
        {
            "code" : "BUCKET_CLOSED_SUCCESSFULLY"
        }
    );
    sharp_client.add_mock_on_endpoint(
        "/close_bucket",
        vec!["".to_string()],
        Some(200),
        MockResponseBodyType::Json(close_bucket_response),
    );
}

/// Mocks the starknet get nonce call (happens in da client for ethereum)
pub async fn mock_starknet_get_nonce(starknet_client: &mut StarknetClient, l2_block_number: String) {
    let mut file = File::open(format!("artifacts/nonces_{}.json", l2_block_number)).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    #[derive(Deserialize, Debug, Serialize)]
    struct NonceAddress {
        nonce: String,
        address: String,
    }

    // Parse the JSON string into a HashMap
    let vec: Vec<NonceAddress> = serde_json::from_str(&contents).unwrap();

    for ele in vec {
        let address = Felt::from_dec_str(&ele.address).expect("Failed to convert to felt");
        let hex_field_element = vec_u8_to_hex_string(&address.to_bytes_be());

        let response = json!({ "id": 640641,"jsonrpc":"2.0","result": ele.nonce });
        starknet_client.add_mock_on_endpoint(
            "/",
            vec!["starknet_getNonce".to_string(), hex_field_element],
            Some(200),
            MockResponseBodyType::Json(response),
        );
    }
}

/// Mocks the starknet get state update call (happens in da client for ethereum)
pub async fn mock_starknet_get_state_update(starknet_client: &mut StarknetClient, l2_block_number: String) {
    let state_update = read_state_update_from_file(&format!("artifacts/get_state_update_{}.json", l2_block_number))
        .expect("issue while reading");

    let state_update = MaybePendingStateUpdate::Update(state_update);
    let state_update = serde_json::to_value(state_update).unwrap();
    let response = json!({ "id": 640641,"jsonrpc":"2.0","result": state_update });

    starknet_client.add_mock_on_endpoint(
        "/",
        vec!["starknet_getStateUpdate".to_string()],
        Some(200),
        MockResponseBodyType::Json(response),
    );
}

/// Mocks the starknet get state update call (happens in da client for ethereum)
pub async fn mock_starknet_get_latest_block(starknet_client: &mut StarknetClient, l2_block_number: String) {
    starknet_client.add_mock_on_endpoint(
        "/",
        vec!["starknet_blockNumber".to_string()],
        Some(200),
        MockResponseBodyType::Json(json!({
                "id": 640641,"jsonrpc":"2.0","result": l2_block_number.parse::<u64>().unwrap()
        })),
    );
}

// --------------------------- Adding data to database ---------------------------

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_snos(mongo_db: &MongoDbServer, l2_block_number: String) -> Uuid {
    // Create the SNOS-specific metadata
    let snos_metadata = SnosMetadata {
        block_number: l2_block_number.parse().expect("Invalid block number"),
        full_output: false,
        cairo_pie_path: Some(format!("{}/{}", &l2_block_number, CAIRO_PIE_FILE_NAME)),
        on_chain_data_path: Some(format!("{}/{}", &l2_block_number, ON_CHAIN_DATA_FILE_NAME)),
        snos_output_path: Some(format!("{}/{}", &l2_block_number, SNOS_OUTPUT_FILE_NAME)),
        program_output_path: Some(format!("{}/{}", &l2_block_number, PROGRAM_OUTPUT_FILE_NAME)),
        snos_fact: None,
        snos_n_steps: None,
    };

    // Create the common metadata with default values
    let common_metadata = CommonMetadata::default();

    // Combine into JobMetadata
    let metadata = JobMetadata { common: common_metadata, specific: JobSpecificMetadata::Snos(snos_metadata) };

    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: l2_block_number.clone(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client.database("orchestrator").drop(None).await.unwrap();
    mongo_db_client
        .database("orchestrator")
        .collection(JOBS_COLLECTION)
        .insert_one(job_item.clone(), None)
        .await
        .expect("Failed to insert SNOS job into database");

    job_item.id
}

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_da(mongo_db: &MongoDbServer, l2_block_number: String) {
    // Create the DA-specific metadata
    let da_metadata = DaMetadata {
        block_number: l2_block_number.parse::<u64>().unwrap() - 1,
        blob_data_path: Some(format!("{}/{}", l2_block_number.clone(), BLOB_DATA_FILE_NAME)),
        tx_hash: None,
    };

    // Create the common metadata with default values
    let common_metadata = CommonMetadata::default();

    // Combine into JobMetadata
    let metadata = JobMetadata { common: common_metadata, specific: JobSpecificMetadata::Da(da_metadata) };

    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: (l2_block_number.parse::<u32>().unwrap() - 1).to_string(),
        job_type: JobType::DataSubmission,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client
        .database("orchestrator")
        .collection(JOBS_COLLECTION)
        .insert_one(job_item, None)
        .await
        .expect("Failed to insert DA job into database");
}

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_update_state(mongo_db: &MongoDbServer, l2_block_number: String) {
    let block_number = l2_block_number.parse::<u64>().unwrap() - 1;

    // Create the StateUpdate-specific metadata
    let state_update_metadata = StateUpdateMetadata {
        snos_output_paths: vec![format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)],
        program_output_paths: vec![format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)],
        blob_data_paths: vec![format!("{}/{}", block_number, BLOB_DATA_FILE_NAME)],
        tx_hashes: Vec::new(),
        context: SettlementContext::Block(SettlementContextData { to_settle: vec![block_number], last_failed: None }),
    };

    // Create the common metadata with default values
    let common_metadata = CommonMetadata::default();

    // Combine into JobMetadata
    let metadata =
        JobMetadata { common: common_metadata, specific: JobSpecificMetadata::StateUpdate(state_update_metadata) };

    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: block_number.to_string(),
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client
        .database("orchestrator")
        .collection(JOBS_COLLECTION)
        .insert_one(job_item, None)
        .await
        .expect("Failed to insert Update State job into database");
}

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_proving(mongo_db: &MongoDbServer, l2_block_number: String) {
    let block_number = l2_block_number.parse::<u64>().unwrap() - 1;

    // Create the Proving-specific metadata
    let proving_metadata = ProvingMetadata { block_number, ..Default::default() };
    let common_metadata = CommonMetadata::default();

    // Combine into JobMetadata
    let metadata = JobMetadata { common: common_metadata, specific: JobSpecificMetadata::Proving(proving_metadata) };

    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: block_number.to_string(),
        job_type: JobType::ProofCreation,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client
        .database("orchestrator")
        .collection(JOBS_COLLECTION)
        .insert_one(job_item, None)
        .await
        .expect("Failed to insert Proving job into database");
}
