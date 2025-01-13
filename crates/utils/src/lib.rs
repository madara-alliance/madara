pub mod collections;
pub mod env_utils;
pub mod http_client;
pub mod metrics;

/// Evaluate `$x:expr` and if not true return `Err($y:expr)`.
///
/// Used as `ensure!(expression_to_ensure, expression_to_return_on_false)`.
#[macro_export]
macro_rules! ensure {
    ($x:expr, $y:expr $(,)?) => {{
        if !$x {
            return Err($y);
        }
    }};
}
