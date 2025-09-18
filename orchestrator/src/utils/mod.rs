pub mod constants;
pub mod helpers;
pub mod instrument;
pub mod job_status_metrics;
pub mod logging;
pub mod metrics;
pub mod metrics_recorder;
pub mod signal_handler;

/// TODO: This is super Awkward to have this code here
/// but will try to remove this and move it to the config from the root path
pub const COMPILED_OS: &[u8] = include_bytes!("../../../build-artifacts/cairo_lang/os_latest.json");
