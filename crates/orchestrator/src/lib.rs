#![feature(duration_constructors)]

/// Contains the trait implementations for alerts
pub mod alerts;
/// Config of the service. Contains configurations for DB, Queues and other services.
pub mod config;
pub mod constants;
/// Controllers for the routes
pub mod controllers;
pub mod cron;
/// Contains the trait that implements the fetching functions
/// for blob and SNOS data from cloud for a particular block.
pub mod data_storage;
/// Contains the trait that all database clients must implement
pub mod database;
/// Contains the trait that all jobs must implement. Also
/// contains the root level functions for which detect the job
/// type and call the corresponding job
pub mod jobs;
/// contains trait level metrics.
pub mod metrics;
/// Contains the trait that all queues must implement
pub mod queue;
/// Contains the routes for the service
pub mod routes;
/// Contains setup functions to set up db and cloud.
pub mod setup;
/// Contains telemetry collection services. (Metrics/Logs/Traces)
pub mod telemetry;
#[cfg(test)]
pub mod tests;
/// Contains workers which act like cron jobs
pub mod workers;

/// Contains the CLI arguments for the service
pub mod cli;
