/// Gateway Health Tracking
///
/// This module provides gateway-specific health monitoring by re-exporting the generic
/// mp-resilience ConnectionHealth with gateway-specific type aliases.
use mp_resilience::ConnectionHealth;

/// Gateway health tracker (re-exports generic ConnectionHealth)
pub type GatewayHealth = ConnectionHealth;

// Re-export health utilities for convenience
pub use mp_resilience::{start_health_monitor, HealthState};

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
