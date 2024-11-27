pub mod api;
pub mod methods;

use std::time::SystemTime;

pub use api::*;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
}
