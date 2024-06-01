/// Config of the service. Contains configurations for DB, Queues and other services.
pub mod config;
/// Controllers for the routes
pub mod controllers;
/// Contains the trait that all database clients must implement
pub mod database;
/// Contains the trait that all jobs must implement. Also
/// contains the root level functions for which detect the job
/// type and call the corresponding job
pub mod jobs;
/// Contains the trait that all queues must implement
pub mod queue;
/// Contains the routes for the service
pub mod routes;
/// Contains the utils
pub mod utils;
/// Contains workers which act like cron jobs
pub mod workers;

#[cfg(test)]
mod tests;
