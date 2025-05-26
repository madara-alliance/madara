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
use orchestrator::core::client::SQS;
use orchestrator::types::constant::{
    BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use orchestrator::types::jobs::external_id::ExternalId;
use orchestrator::types::jobs::job_item::JobItem;
use orchestrator::types::jobs::metadata::{
    CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata, SnosMetadata, StateUpdateMetadata,
};
use orchestrator::types::jobs::types::{JobStatus, JobType};
use orchestrator::types::params::database::DatabaseArgs;
use orchestrator::types::params::QueueArgs;
use orchestrator::types::queue::QueueType;
use orchestrator::worker::parser::job_queue_message::JobQueueMessage;
use orchestrator_utils::env_utils::get_env_var_or_panic;
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
        env_vec.insert("MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS".to_string(), l2_block_number.clone());
        env_vec.insert("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS".to_string(), l2_block_number);

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
#[case("66645".to_string())]
#[tokio::test]
async fn test_orchestrator_workflow(#[case] l2_block_number: String) {
    // Fetching the env vars from the test env file as these will be used in
    // setting up of the test and during orchestrator run too.

    use e2e_tests::node::OrchestratorMode;
    println!("Loading .env file");
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");

    let queue_params = QueueArgs {
        prefix: get_env_var_or_panic("MADARA_ORCHESTRATOR_AWS_PREFIX"),
        suffix: get_env_var_or_panic("MADARA_ORCHESTRATOR_SQS_SUFFIX"),
    };

    let mut setup_config = Setup::new(l2_block_number.clone()).await;
    // Setup Cloud
    // Setup orchestrator cloud
    Orchestrator::new(OrchestratorMode::Setup, setup_config.envs());
    println!("✅ Orchestrator cloud setup completed");

    // Step 1 : SNOS job runs =========================================
    // Updates the job in the db
    let job_id = put_job_data_in_db_snos(setup_config.mongo_db_instance(), l2_block_number.clone()).await;
    put_snos_job_in_processing_queue(job_id, queue_params).await.unwrap();

    // Step 2: Proving Job ============================================
    // Mocking the endpoint
    mock_proving_job_endpoint_output(setup_config.sharp_client()).await;
    put_job_data_in_db_proving(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    // Step 3: DA job =================================================
    // Adding a mock da job so that worker does not create 60k+ jobs
    put_job_data_in_db_da(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    // Step 4: State Update job =======================================
    put_job_data_in_db_update_state(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    println!("✅ Orchestrator setup completed.");

    // Run orchestrator
    let mut orchestrator =
        Orchestrator::new(OrchestratorMode::Run, setup_config.envs()).expect("Failed to start orchestrator");
    orchestrator.wait_till_started().await;

    println!("✅ Orchestrator started");

    // Adding State checks in DB for validation of tests

    // Check 1 : After Proving Job state (15 mins. approx time)
    let expected_state_after_proving_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::ProofCreation,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(900),
        l2_block_number.clone(),
        setup_config.mongo_db_instance(),
        expected_state_after_proving_job,
    )
    .await;
    assert!(test_result.is_ok(), "After Proving Job state DB state assertion failed.");

    // Check 2 : After DA Job state (5 mins. approx time)
    let expected_state_after_da_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::DataSubmission,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(300),
        l2_block_number.clone(),
        setup_config.mongo_db_instance(),
        expected_state_after_da_job,
    )
    .await;
    assert!(test_result.is_ok(), "After DA Job state DB state assertion failed.");

    // Check 3 : After Update State Job state (5 mins. approx time)
    let expected_state_after_da_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::StateTransition,
        job_status: JobStatus::Completed,
        version: 4,
    };
    let test_result = wait_for_db_state(
        Duration::from_secs(300),
        l2_block_number,
        setup_config.mongo_db_instance(),
        expected_state_after_da_job,
    )
    .await;
    assert!(test_result.is_ok(), "After Update State Job state DB state assertion failed.");
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
            get_database_state(mongo_db_server, l2_block_for_testing.clone(), expected_db_state.job_type.clone())
                .await
                .unwrap();

        if db_state.is_some() && db_state.unwrap() == expected_db_state {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(format!("Timed out waiting for expected state: {:?}", expected_db_state))
}

/// Fetch the job from database
async fn get_database_state(
    mongo_db_server: &MongoDbServer,
    l2_block_for_testing: String,
    job_type: JobType,
) -> color_eyre::Result<Option<ExpectedDBState>> {
    let mongo_db_client = get_mongo_db_client(mongo_db_server).await;
    let collection = mongo_db_client.database("orchestrator").collection::<JobItem>("jobs");
    let filter = doc! { "internal_id": l2_block_for_testing, "job_type" : mongodb::bson::to_bson(&job_type)? };
    let job = collection.find_one(filter, None).await.unwrap();
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

// ======================================
// Util functions
// ======================================

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_snos(mongo_db: &MongoDbServer, l2_block_number: String) -> Uuid {
    // Create the SNOS-specific metadata
    let snos_metadata = SnosMetadata {
        block_number: l2_block_number.parse().expect("Invalid block number"),
        full_output: false,
        cairo_pie_path: Some(format!("{}/{}", l2_block_number.clone(), CAIRO_PIE_FILE_NAME)),
        snos_output_path: Some(format!("{}/{}", l2_block_number.clone(), SNOS_OUTPUT_FILE_NAME)),
        program_output_path: Some(format!("{}/{}", l2_block_number.clone(), PROGRAM_OUTPUT_FILE_NAME)),
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
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item.clone(), None).await.unwrap();

    job_item.id
}

/// Adding SNOS job in JOB_PROCESSING_QUEUE so that the job is triggered
/// as soon as it is picked up by orchestrator
pub async fn put_snos_job_in_processing_queue(id: Uuid, queue_params: QueueArgs) -> color_eyre::Result<()> {
    let message = JobQueueMessage { id };

    let config = aws_config::from_env().load().await;
    let queue = SQS::new(&config, Some(&queue_params));
    let queue_name = format!("{}_{}_{}", queue_params.prefix, QueueType::SnosJobProcessing, queue_params.suffix);
    let queue_url = queue.get_queue_url_from_client(queue_name.as_str()).await?;
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

/// Mocks the endpoint for sharp client
pub async fn mock_proving_job_endpoint_output(sharp_client: &mut SharpClient) {
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
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item, None).await.unwrap();
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

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_update_state(mongo_db: &MongoDbServer, l2_block_number: String) {
    let block_number = l2_block_number.parse::<u64>().unwrap() - 1;

    // Create the StateUpdate-specific metadata
    let state_update_metadata = StateUpdateMetadata {
        blocks_to_settle: vec![block_number],
        snos_output_paths: vec![format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)],
        program_output_paths: vec![format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)],
        blob_data_paths: vec![format!("{}/{}", block_number, BLOB_DATA_FILE_NAME)],
        last_failed_block_no: None,
        tx_hashes: Vec::new(),
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
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item, None).await.unwrap();
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
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item, None).await.unwrap();
}
