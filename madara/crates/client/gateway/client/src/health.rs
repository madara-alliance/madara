/// Gateway Health Tracking
///
/// This module provides gateway-specific health monitoring by wrapping the generic
/// mp-resilience ConnectionHealth with gateway-specific context.

use mp_resilience::ConnectionHealth;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Gateway health tracker (wraps generic ConnectionHealth)
pub type GatewayHealth = ConnectionHealth;

/// Start the background health monitor task for the gateway
///
/// This spawns a tokio task that periodically logs gateway health status.
/// The task will run until the health Arc is dropped or the program exits.
///
/// # Arguments
/// * `health` - Arc to the GatewayHealth instance to monitor
pub fn start_gateway_health_monitor(health: Arc<RwLock<GatewayHealth>>) {
    mp_resilience::start_health_monitor(health);
}

// Re-export HealthState for convenience
pub use mp_resilience::HealthState;

#[cfg(test)]
mod tests {
    use super::*;
    use mp_resilience::HealthState;

    #[test]
    fn test_gateway_health_creation() {
        let health = GatewayHealth::new("Gateway");
        assert!(matches!(health.state(), HealthState::Healthy));
    }
}
