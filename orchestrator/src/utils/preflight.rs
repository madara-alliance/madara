use crate::core::client::queue::QueueClient;
use crate::core::client::storage::StorageClient;
use crate::core::client::{AlertClient, DatabaseClient};
use crate::error::OrchestratorError;
use crate::OrchestratorResult;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{error, info};

/// Default timeout for each health check (in seconds)
const DEFAULT_HEALTH_CHECK_TIMEOUT_SECS: u64 = 30;

/// Health check results for tracking which components passed/failed
#[derive(Debug, Default)]
pub struct HealthCheckResults {
    pub mongodb_passed: bool,
    pub s3_passed: bool,
    pub sqs_passed: bool,
    pub sns_passed: bool,
}

impl HealthCheckResults {
    /// Check if all health checks passed
    pub fn all_passed(&self) -> bool {
        self.mongodb_passed && self.s3_passed && self.sqs_passed && self.sns_passed
    }

    /// Get a summary of failed checks
    pub fn failed_checks(&self) -> Vec<&str> {
        let mut failed = Vec::new();
        if !self.mongodb_passed {
            failed.push("MongoDB");
        }
        if !self.s3_passed {
            failed.push("AWS S3");
        }
        if !self.sqs_passed {
            failed.push("AWS SQS");
        }
        if !self.sns_passed {
            failed.push("AWS SNS");
        }
        failed
    }
}

/// Runs comprehensive pre-flight checks for all critical resources
///
/// This function validates that all external dependencies are accessible
/// before the application starts processing. It follows a fail-fast approach
/// and provides clear error messages for debugging.
///
/// # Arguments
/// * `database` - Database client (MongoDB)
/// * `storage` - Storage client (AWS S3)
/// * `queue` - Queue client (AWS SQS)
/// * `alerts` - Alert client (AWS SNS)
///
/// # Returns
/// * `Ok(())` - If all health checks pass
/// * `Err(_)` - If any health check fails, with details about the failure
pub async fn run_preflight_checks(
    database: &dyn DatabaseClient,
    storage: &dyn StorageClient,
    queue: &dyn QueueClient,
    alerts: &dyn AlertClient,
) -> OrchestratorResult<()> {
    info!("🚀 Starting pre-flight health checks...");
    info!("⏱️  Health check timeout: {}s per service", DEFAULT_HEALTH_CHECK_TIMEOUT_SECS);

    // Run all health checks
    let results = HealthCheckResults {
        mongodb_passed: check_mongodb_health(database).await,
        s3_passed: check_s3_health(storage).await,
        sqs_passed: check_sqs_health(queue).await,
        sns_passed: check_sns_health(alerts).await,
    };

    // Evaluate results
    if results.all_passed() {
        info!("✅ All pre-flight health checks passed successfully");
        Ok(())
    } else {
        let failed = results.failed_checks();
        error!(
            failed_services = ?failed,
            "❌ Pre-flight health checks failed"
        );

        Err(OrchestratorError::RunCommandError(format!(
            "Pre-flight health checks failed for the following services: {}. \
            Please ensure these services are accessible and properly configured.",
            failed.join(", ")
        )))
    }
}

/// Check MongoDB connectivity and basic operations
async fn check_mongodb_health(database: &dyn DatabaseClient) -> bool {
    info!("🔍 Checking MongoDB connectivity...");

    let check_result = timeout(Duration::from_secs(DEFAULT_HEALTH_CHECK_TIMEOUT_SECS), async {
        // Try to perform a basic database operation
        // This will test both connectivity and authentication
        database.health_check().await
    })
    .await;

    match check_result {
        Ok(Ok(_)) => {
            info!("✅ MongoDB health check passed");
            true
        }
        Ok(Err(e)) => {
            error!(
                error = %e,
                "❌ MongoDB health check failed"
            );
            false
        }
        Err(_) => {
            error!(timeout_secs = DEFAULT_HEALTH_CHECK_TIMEOUT_SECS, "❌ MongoDB health check timed out");
            false
        }
    }
}

/// Check AWS S3 accessibility and permissions
async fn check_s3_health(storage: &dyn StorageClient) -> bool {
    info!("🔍 Checking AWS S3 accessibility...");

    let check_result =
        timeout(Duration::from_secs(DEFAULT_HEALTH_CHECK_TIMEOUT_SECS), async { storage.health_check().await }).await;

    match check_result {
        Ok(Ok(_)) => {
            info!("✅ AWS S3 health check passed");
            true
        }
        Ok(Err(e)) => {
            error!(
                error = %e,
                "❌ AWS S3 health check failed"
            );
            false
        }
        Err(_) => {
            error!(timeout_secs = DEFAULT_HEALTH_CHECK_TIMEOUT_SECS, "❌ AWS S3 health check timed out");
            false
        }
    }
}

/// Check AWS SQS accessibility and permissions
async fn check_sqs_health(queue: &dyn QueueClient) -> bool {
    info!("🔍 Checking AWS SQS accessibility...");

    let check_result =
        timeout(Duration::from_secs(DEFAULT_HEALTH_CHECK_TIMEOUT_SECS), async { queue.health_check().await }).await;

    match check_result {
        Ok(Ok(_)) => {
            info!("✅ AWS SQS health check passed");
            true
        }
        Ok(Err(e)) => {
            error!(
                error = %e,
                "❌ AWS SQS health check failed"
            );
            false
        }
        Err(_) => {
            error!(timeout_secs = DEFAULT_HEALTH_CHECK_TIMEOUT_SECS, "❌ AWS SQS health check timed out");
            false
        }
    }
}

/// Check AWS SNS accessibility and permissions
async fn check_sns_health(alerts: &dyn AlertClient) -> bool {
    info!("🔍 Checking AWS SNS accessibility...");

    let check_result =
        timeout(Duration::from_secs(DEFAULT_HEALTH_CHECK_TIMEOUT_SECS), async { alerts.health_check().await }).await;

    match check_result {
        Ok(Ok(_)) => {
            info!("✅ AWS SNS health check passed");
            true
        }
        Ok(Err(e)) => {
            error!(
                error = %e,
                "❌ AWS SNS health check failed"
            );
            false
        }
        Err(_) => {
            error!(timeout_secs = DEFAULT_HEALTH_CHECK_TIMEOUT_SECS, "❌ AWS SNS health check timed out");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_check_results_all_passed() {
        let mut results = HealthCheckResults::default();
        assert!(!results.all_passed());

        results.mongodb_passed = true;
        results.s3_passed = true;
        results.sqs_passed = true;
        results.sns_passed = true;
        assert!(results.all_passed());
    }

    #[test]
    fn test_health_check_results_failed_checks() {
        let results = HealthCheckResults { mongodb_passed: true, sqs_passed: true, ..Default::default() };
        let failed = results.failed_checks();
        assert_eq!(failed.len(), 2);
        assert!(failed.contains(&"AWS S3"));
        assert!(failed.contains(&"AWS SNS"));
    }
}
