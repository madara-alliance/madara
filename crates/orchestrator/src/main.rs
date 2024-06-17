use dotenvy::dotenv;
use orchestrator::config::config;
use orchestrator::queue::init_consumers;
use orchestrator::routes::app_router;
use orchestrator::utils::env_utils::get_env_var_or_default;
use orchestrator::workers::proof_registration::ProofRegistrationWorker;
use orchestrator::workers::proving::ProvingWorker;
use orchestrator::workers::snos::SnosWorker;
use orchestrator::workers::update_state::UpdateStateWorker;
use orchestrator::workers::*;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).init();

    // initial config setup
    config().await;
    let host = get_env_var_or_default("HOST", "127.0.0.1");
    let port = get_env_var_or_default("PORT", "3000").parse::<u16>().expect("PORT must be a u16");
    let address = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let app = app_router();

    // init consumer
    init_consumers().await.expect("Failed to init consumers");

    // spawn a thread for each workers
    // changes in rollup mode - sovereign, validity, validiums etc.
    // will likely involve changes in these workers as well
    tokio::spawn(start_cron(Box::new(SnosWorker), 60));
    tokio::spawn(start_cron(Box::new(ProvingWorker), 60));
    tokio::spawn(start_cron(Box::new(ProofRegistrationWorker), 60));
    tokio::spawn(start_cron(Box::new(UpdateStateWorker), 60));

    tracing::info!("Listening on http://{}", address);
    axum::serve(listener, app).await.expect("Failed to start axum server");
}

async fn start_cron(worker: Box<dyn Worker>, interval: u64) {
    loop {
        worker.run_worker().await.expect("Error in running the worker.");
        tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
    }
}
