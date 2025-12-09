/// Job retry and timeout policies configuration
///
/// This module defines configurable policies for each job type, replacing the previously
/// hardcoded retry attempts, verification attempts, and polling delays.

#[derive(Debug, Clone)]
pub struct JobPolicies {
    pub snos_execution: JobPolicy,
    pub proving: JobPolicy,
    pub da_submission: JobPolicy,
    pub state_update: JobPolicy,
    pub aggregator: JobPolicy,
    pub proof_registration: JobPolicy,
}

#[derive(Debug, Clone)]
pub struct JobPolicy {
    /// Maximum number of attempts to process the job (retries on failure)
    pub max_process_attempts: u64,
    /// Maximum number of attempts to verify the job (polling for completion)
    pub max_verification_attempts: u64,
    /// Delay in seconds between verification polling attempts
    pub verification_polling_delay_seconds: u64,
}

impl Default for JobPolicies {
    fn default() -> Self {
        Self {
            // SNOS execution: fast, no retries (was 1, 1, 1)
            snos_execution: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 1,
                verification_polling_delay_seconds: 1,
            },
            // Proving: no retries, long verification wait (was 1, 300, 30)
            proving: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 30,
            },
            // DA submission: no retries, short verification (was 1, 3, 60)
            da_submission: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 3,
                verification_polling_delay_seconds: 60,
            },
            // State update: no retries, medium verification (was 1, 10, 60)
            state_update: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 10,
                verification_polling_delay_seconds: 60,
            },
            // Aggregator: no retries, long verification (was 1, 300, 30)
            aggregator: JobPolicy {
                max_process_attempts: 1,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 30,
            },
            // Proof registration: allows retries, long verification (was 2, 300, 300)
            proof_registration: JobPolicy {
                max_process_attempts: 2,
                max_verification_attempts: 300,
                verification_polling_delay_seconds: 300,
            },
        }
    }
}
