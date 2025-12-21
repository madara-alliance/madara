/// Worker implementation for queue-less job processing
///
/// Workers poll MongoDB for available jobs and claim them atomically
/// using findOneAndUpdate to prevent race conditions.
pub mod config;
pub mod controller;
pub mod worker;

pub use config::WorkerConfig;
pub use controller::WorkerController;
pub use worker::{Worker, WorkerPhase};
