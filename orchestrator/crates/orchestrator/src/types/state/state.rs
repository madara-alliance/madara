use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SystemState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Error(String),
}
