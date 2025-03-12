/// Contains the CLI arguments for the service
pub mod args;
/// Client for the Orchestrator
// pub mod client;

/// Contains the CLI arguments for the service
pub mod constants;
/// Contain the controllers for the service
pub mod controller;

/// Contains the core logic for the service
pub mod core;

/// contains all the error handling / errors that can be returned by the service
pub mod error;
/// contains all the resources that can be used by the service
pub mod resource;
/// Contains all the services that are used by the service
pub mod service;
pub mod setup;
/// Contains all the utils that are used by the service
pub mod utils;

use crate::args::Cli;
use clap::Parser as _;
use dotenvy::dotenv;
use orchestrator::utils::logging::init_logging;

/// Start the server
#[tokio::main]
#[allow(clippy::needless_return)]
async fn main() {
    dotenv().ok();
    let cli = Cli::parse();
    init_logging().expect("Failed to initialize logging");
    color_eyre::install().expect("Unable to install color_eyre");
    tracing::info!("Starting orchestrator");
}
