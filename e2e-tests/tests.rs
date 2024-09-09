use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use e2e_tests::localstack::LocalStack;
use e2e_tests::sharp::SharpClient;
use e2e_tests::starknet_client::StarknetClient;
use e2e_tests::utils::{get_mongo_db_client, read_state_update_from_file, vec_u8_to_hex_string};
use e2e_tests::{MongoDbServer, Orchestrator};
use ethereum_settlement_client::tests::{EthereumTest, EthereumTestBuilder, STARKNET_OPERATOR_ADDRESS};
use mongodb::bson::doc;
use orchestrator::data_storage::DataStorage;
use orchestrator::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use orchestrator::queue::job_queue::WorkerTriggerType;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use serde_json::json;
use starknet::core::types::{FieldElement, MaybePendingStateUpdate};
use std::collections::HashMap;
use std::fs::{read, File};
use std::io::Read;
use std::str::FromStr;
use std::time::{Duration, Instant};
use utils::env_utils::get_env_var_or_panic;
use uuid::Uuid;

extern crate e2e_tests;

/// Expected DB state struct
#[derive(PartialEq, Debug)]
struct ExpectedDBState {
    internal_id: String,
    job_type: JobType,
    job_status: JobStatus,
    version: i32,
}

/// Initial setup for e2e tests
struct Setup {
    mongo_db_instance: MongoDbServer,
    starknet_client: StarknetClient,
    _ethereum_client: EthereumTest,
    sharp_client: SharpClient,
    env_vector: Vec<(String, String)>,
    localstack_instance: LocalStack,
}

const L1_BLOCK_TO_FORK: u64 = 20607627;

impl Setup {
    /// Initialise a new setup
    pub async fn new() -> Self {
        let mongo_db_instance = MongoDbServer::run().await;
        println!("✅ Mongo DB setup completed");
        let starknet_client = StarknetClient::new();
        println!("✅ Starknet/Madara client setup completed");
        let ethereum_client = EthereumTestBuilder::new()
            .with_fork_block(L1_BLOCK_TO_FORK)
            .with_impersonator(*STARKNET_OPERATOR_ADDRESS)
            .build()
            .await;
        println!("✅ Ethereum client setup completed");

        let sharp_client = SharpClient::new();
        println!("✅ Sharp client setup completed");

        // Setting up LocalStack
        let localstack_instance = LocalStack::new().await;
        localstack_instance.setup_sqs().await.unwrap();
        localstack_instance.delete_event_bridge_rule("worker_trigger_scheduled").await.unwrap();
        localstack_instance.setup_event_bridge(WorkerTriggerType::Proving).await.unwrap();
        localstack_instance.setup_event_bridge(WorkerTriggerType::DataSubmission).await.unwrap();
        localstack_instance.setup_event_bridge(WorkerTriggerType::UpdateState).await.unwrap();

        println!("✅ Localstack instance setup completed");

        let mut env_vec: Vec<(String, String)> =
            vec![("MONGODB_CONNECTION_STRING".to_string(), mongo_db_instance.endpoint().to_string())];

        // Adding other values to the environment variables vector
        env_vec.push(("MADARA_RPC_URL".to_string(), starknet_client.url()));
        env_vec.push(("SETTLEMENT_RPC_URL".to_string(), ethereum_client.rpc_url.clone().to_string()));
        env_vec.push(("SHARP_URL".to_string(), sharp_client.url()));

        // Sharp envs
        env_vec.push(("SHARP_CUSTOMER_ID".to_string(), get_env_var_or_panic("SHARP_CUSTOMER_ID")));
        env_vec.push(("SHARP_USER_CRT".to_string(), get_env_var_or_panic("SHARP_USER_CRT")));
        env_vec.push(("SHARP_USER_KEY".to_string(), get_env_var_or_panic("SHARP_USER_KEY")));
        env_vec.push(("SHARP_SERVER_CRT".to_string(), get_env_var_or_panic("SHARP_SERVER_CRT")));

        Self {
            mongo_db_instance,
            starknet_client,
            _ethereum_client: ethereum_client,
            sharp_client,
            env_vector: env_vec,
            localstack_instance,
        }
    }

    pub fn mongo_db_instance(&self) -> &MongoDbServer {
        &self.mongo_db_instance
    }

    pub fn starknet_client(&mut self) -> &mut StarknetClient {
        &mut self.starknet_client
    }

    pub fn sharp_client(&mut self) -> &mut SharpClient {
        &mut self.sharp_client
    }

    pub fn envs(&self) -> Vec<(String, String)> {
        self.env_vector.clone()
    }

    pub fn localstack(&self) -> &LocalStack {
        &self.localstack_instance
    }
}

#[rstest]
#[case("671070".to_string())]
#[tokio::test]
async fn test_orchestrator_workflow(#[case] l2_block_number: String) {
    // Fetching the env vars from the test env file as these will be used in
    // setting up of the test and during orchestrator run too.
    dotenvy::from_filename(".env.test").expect("Failed to load the .env file");

    let mut setup_config = Setup::new().await;
    // Setup S3
    setup_s3(setup_config.localstack().s3_client(), l2_block_number.clone()).await.unwrap();

    // Step 1 : SNOS job runs =========================================
    // TODO : Update the code with actual SNOS implementation
    // Updates the job in the db
    put_job_data_in_db_snos(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    // Step 2: Proving Job ============================================
    // Mocking the endpoint
    mock_proving_job_endpoint_output(setup_config.sharp_client()).await;

    // Step 3: DA job =================================================
    // mocking get_block_call from starknet client

    // Adding a mock da job so that worker does not create 60k+ jobs
    put_job_data_in_db_da(setup_config.mongo_db_instance(), l2_block_number.clone()).await;
    mock_starknet_get_state_update(setup_config.starknet_client(), l2_block_number.clone()).await;
    mock_starknet_get_nonce(setup_config.starknet_client(), l2_block_number.clone()).await;

    // Step 4: State Update job =======================================
    put_job_data_in_db_update_state(setup_config.mongo_db_instance(), l2_block_number.clone()).await;

    println!("✅ Orchestrator setup completed.");

    // Run orchestrator
    let mut orchestrator = Orchestrator::run(setup_config.envs());
    orchestrator.wait_till_started().await;

    // Adding State checks in DB for validation of tests

    // Check 1 : After Proving Job state (15 mins. approx time)
    let expected_state_after_proving_job = ExpectedDBState {
        internal_id: l2_block_number.clone(),
        job_type: JobType::ProofCreation,
        job_status: JobStatus::Completed,
        version: 3,
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
        version: 3,
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
        version: 3,
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
pub async fn put_job_data_in_db_snos(mongo_db: &MongoDbServer, l2_block_number: String) {
    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: l2_block_number,
        job_type: JobType::SnosRun,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata: HashMap::new(),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client.database("orchestrator").drop(None).await.unwrap();
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item, None).await.unwrap();
}

/// Mocks the endpoint for sharp client
pub async fn mock_proving_job_endpoint_output(sharp_client: &mut SharpClient) {
    // Add job response
    let add_job_response = json!(
        {
            "code" : "JOB_RECEIVED_SUCCESSFULLY"
        }
    );
    sharp_client.add_mock_on_endpoint("/add_job", vec!["".to_string()], Some(200), &add_job_response);

    // Getting job response
    let get_job_response = json!(
        {
                "status": "ONCHAIN",
                "validation_done": true
        }
    );
    sharp_client.add_mock_on_endpoint("/get_status", vec!["".to_string()], Some(200), &get_job_response);
}

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_da(mongo_db: &MongoDbServer, l2_block_number: String) {
    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: (l2_block_number.parse::<u32>().unwrap() - 1).to_string(),
        job_type: JobType::DataSubmission,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata: HashMap::new(),
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
        let address = FieldElement::from_str(&ele.address).unwrap();
        let hex_field_element = vec_u8_to_hex_string(&address.to_bytes_be());

        let response = json!({ "id": 640641,"jsonrpc":"2.0","result": ele.nonce });
        starknet_client.add_mock_on_endpoint(
            "/",
            vec!["starknet_getNonce".to_string(), hex_field_element],
            Some(200),
            &response,
        );
    }
}

/// Mocks the starknet get state update call (happens in da client for ethereum)
pub async fn mock_starknet_get_state_update(starknet_client: &mut StarknetClient, l2_block_number: String) {
    let state_update = read_state_update_from_file(&format!("artifacts/get_state_update_{}.json", l2_block_number))
        .expect("issue while reading");

    let state_update = MaybePendingStateUpdate::Update(state_update);
    let state_update = serde_json::to_value(&state_update).unwrap();
    let response = json!({ "id": 640641,"jsonrpc":"2.0","result": state_update });

    starknet_client.add_mock_on_endpoint("/", vec!["starknet_getStateUpdate".to_string()], Some(200), &response);
}

/// Puts after SNOS job state into the database
pub async fn put_job_data_in_db_update_state(mongo_db: &MongoDbServer, l2_block_number: String) {
    let job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: (l2_block_number.parse::<u32>().unwrap() - 1).to_string(),
        job_type: JobType::StateTransition,
        status: JobStatus::Completed,
        external_id: ExternalId::Number(0),
        metadata: HashMap::new(),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let mongo_db_client = get_mongo_db_client(mongo_db).await;
    mongo_db_client.database("orchestrator").collection("jobs").insert_one(job_item, None).await.unwrap();
}

// ======================================
// Tests specific functions
// ======================================

/// To set up s3 files needed for e2e test (test_orchestrator_workflow)
#[allow(clippy::borrowed_box)]
pub async fn setup_s3(
    s3_client: &Box<dyn DataStorage + Send + Sync>,
    l2_block_number: String,
) -> color_eyre::Result<()> {
    s3_client.build_test_bucket(&get_env_var_or_panic("AWS_S3_BUCKET_NAME")).await.unwrap();

    // putting the snos output and program output for the given block into localstack s3
    let snos_output_key = l2_block_number.clone() + "/snos_output.json";
    let snos_output_json = read("artifacts/snos_output.json").unwrap();
    s3_client.put_data(Bytes::from(snos_output_json), &snos_output_key).await?;
    println!("✅ snos output file uploaded to localstack s3.");

    let program_output_key = l2_block_number.clone() + "/program_output.txt";
    let program_output = read(format!("artifacts/program_output_{}.txt", l2_block_number.clone())).unwrap();
    s3_client.put_data(Bytes::from(program_output), &program_output_key).await?;
    println!("✅ program output file uploaded to localstack s3.");

    // getting the PIE file from s3 bucket using URL provided
    let file =
        reqwest::get(format!("https://madara-orchestrator-sharp-pie.s3.amazonaws.com/{}-SN.zip", l2_block_number))
            .await?;
    let file_bytes = file.bytes().await?;

    // putting the pie file into localstack s3
    let s3_file_key = l2_block_number + "/pie.zip";
    s3_client.put_data(file_bytes, &s3_file_key).await?;
    println!("✅ PIE file uploaded to localstack s3");

    Ok(())
}
