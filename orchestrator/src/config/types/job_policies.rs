use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPoliciesConfig {
    pub snos_execution: JobPolicy,
    pub proving: JobPolicy,
    pub da_submission: JobPolicy,
    pub state_update: JobPolicy,
    pub aggregator: JobPolicy,
    pub proof_registration: JobPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPolicy {
    /// Maximum number of times to attempt processing before marking as failed
    pub max_process_attempts: u64,

    /// Maximum number of times to poll for verification
    pub max_verification_attempts: u64,

    /// Delay in seconds between verification polls
    pub verification_polling_delay_seconds: u64,

    /// Total timeout for job completion (in seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl JobPolicy {
    pub fn verification_polling_delay(&self) -> Duration {
        Duration::from_secs(self.verification_polling_delay_seconds)
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.timeout_seconds.map(Duration::from_secs)
    }

    pub fn max_wait_time(&self) -> Duration {
        Duration::from_secs(self.max_verification_attempts * self.verification_polling_delay_seconds)
    }
}

impl Default for JobPoliciesConfig {
    fn default() -> Self {
        Self {
            snos_execution: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 1,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(300),
            },
            proving: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 30,
                timeout_seconds: Some(9000),
            },
            da_submission: JobPolicy {
                max_process_attempts: 3,
                max_verification_attempts: 10,
                verification_polling_delay_seconds: 30,
                timeout_seconds: Some(600),
            },
            state_update: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 20,
                verification_polling_delay_seconds: 30,
                timeout_seconds: Some(1200),
            },
            aggregator: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 30,
                timeout_seconds: Some(9000),
            },
            proof_registration: JobPolicy {
                max_process_attempts: 2,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 300,
                timeout_seconds: Some(90000),
            },
        }
    }
}

impl JobPoliciesConfig {
    /// Fast policies for testing (shorter timeouts and delays)
    pub fn test_fast() -> Self {
        Self {
            snos_execution: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 1,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(10),
            },
            proving: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 5,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(30),
            },
            da_submission: JobPolicy {
                max_process_attempts: 2,
                max_verification_attempts: 3,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(15),
            },
            state_update: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 5,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(20),
            },
            aggregator: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 5,
                verification_polling_delay_seconds: 1,
                timeout_seconds: Some(30),
            },
            proof_registration: JobPolicy {
                max_process_attempts: 2,
                max_verification_attempts: 5,
                verification_polling_delay_seconds: 5,
                timeout_seconds: Some(60),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policies() {
        let policies = JobPoliciesConfig::default();

        // Verify proving policy has long timeout
        assert_eq!(policies.proving.max_verification_attempts, 300);
        assert_eq!(policies.proving.verification_polling_delay_seconds, 30);

        // Verify DA allows retries
        assert_eq!(policies.da_submission.max_process_attempts, 3);
    }

    #[test]
    fn test_fast_policies() {
        let policies = JobPoliciesConfig::test_fast();

        // All policies should have short timeouts
        assert!(policies.proving.max_verification_attempts <= 5);
        assert!(policies.proving.verification_polling_delay_seconds <= 5);
    }

    #[test]
    fn test_max_wait_time() {
        let policy = JobPolicy {
            max_process_attempts: 1,
            max_verification_attempts: 10,
            verification_polling_delay_seconds: 30,
            timeout_seconds: Some(600),
        };

        // 10 attempts * 30 seconds = 300 seconds
        assert_eq!(policy.max_wait_time(), Duration::from_secs(300));
    }
}
