pub mod batch;
pub mod constant;
pub mod error;
pub mod jobs;
pub mod params;
pub mod q_control;
pub mod queue;
pub mod worker;

#[derive(Debug, Clone, clap::ValueEnum, PartialEq)]
pub enum Layer {
    L2,
    L3,
}
