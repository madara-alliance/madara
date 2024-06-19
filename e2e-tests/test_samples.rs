use e2e_tests::{MongoDbServer, Orchestrator};

extern crate e2e_tests;

#[ignore = "requires DOCKER_HOST set to run"]
#[tokio::test]
async fn test_orchestrator_launches() {
    let mongodb = MongoDbServer::run().await;
    let mut orchestrator = Orchestrator::run(vec![
        // TODO: mock Madara RPC API
        ("MADARA_RPC_URL", "http://localhost"),
        ("MONGODB_CONNECTION_STRING", mongodb.endpoint().as_str()),
    ]);
    orchestrator.wait_till_started().await;
}
