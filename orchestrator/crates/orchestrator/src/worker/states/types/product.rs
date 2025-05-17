use uuid::Uuid;
use serde::{Deserialize, Serialize};

/// Represents the output data from SNOS processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SNOSData {
    pub job_id: Uuid,
    pub block_number: u64,
    pub block_hash: String,
    pub state_root: String,
    pub timestamp: u64,
}

/// Represents verification data for SNOS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SNOSVerificationData {
    pub job_id: Uuid,
    pub is_valid: bool,
    pub verification_hash: String,
    pub verification_timestamp: u64,
}

/// Represents L3 proof data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L3ProofData {
    pub job_id: Uuid,
    pub proof: String,
    pub public_inputs: Vec<String>,
    pub proof_timestamp: u64,
}

/// Represents L2 proof creation data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2ProofData {
    pub job_id: Uuid,
    pub proof: String,
    pub public_inputs: Vec<String>,
    pub verification_key: String,
    pub proof_timestamp: u64,
}

/// Represents L2 data submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2DataSubmissionData {
    pub job_id: Uuid,
    pub transaction_hash: String,
    pub block_number: u64,
    pub submission_timestamp: u64,
}

/// Represents L2 state update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2StateUpdateData {
    pub job_id: Uuid,
    pub new_state_root: String,
    pub previous_state_root: String,
    pub update_timestamp: u64,
}

/// StateProduct represents the output of each state in the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateProduct {
    /// Initial job ID
    JobID(Uuid),
    
    /// SNOS Processing output
    SNOSProcessing(SNOSData),
    
    /// SNOS Verification output
    SNOSVerification(SNOSVerificationData),
    
    /// L3 Proof Creation output
    L3ProofCreation(L3ProofData),
    
    /// L2 Proof Creation Processing output
    L2ProofCreationProcessing(L2ProofData),
    
    /// L2 Proof Creation Verification output
    L2ProofCreationVerification {
        proof_data: L2ProofData,
        is_valid: bool,
        verification_timestamp: u64,
    },
    
    /// L2 Data Submission Processing output
    L2DataSubmissionProcessing(L2DataSubmissionData),
    
    /// L2 Data Submission Verification output
    L2DataSubmissionVerification {
        submission_data: L2DataSubmissionData,
        is_valid: bool,
        verification_timestamp: u64,
    },
    
    /// L2 State Update Processing output
    L2StateUpdateProcessing(L2StateUpdateData),
    
    /// L2 State Update Verification output
    L2StateUpdateVerification {
        state_update: L2StateUpdateData,
        is_valid: bool,
        verification_timestamp: u64,
    },
}

impl StateProduct {
    /// Get the job ID associated with this state product
    pub fn job_id(&self) -> Uuid {
        match self {
            StateProduct::JobID(id) => *id,
            StateProduct::SNOSProcessing(data) => data.job_id,
            StateProduct::SNOSVerification(data) => data.job_id,
            StateProduct::L3ProofCreation(data) => data.job_id,
            StateProduct::L2ProofCreationProcessing(data) => data.job_id,
            StateProduct::L2ProofCreationVerification { proof_data, .. } => proof_data.job_id,
            StateProduct::L2DataSubmissionProcessing(data) => data.job_id,
            StateProduct::L2DataSubmissionVerification { submission_data, .. } => submission_data.job_id,
            StateProduct::L2StateUpdateProcessing(data) => data.job_id,
            StateProduct::L2StateUpdateVerification { state_update, .. } => state_update.job_id,
        }
    }

    /// Get the timestamp associated with this state product
    pub fn timestamp(&self) -> Option<u64> {
        match self {
            StateProduct::JobID(_) => None,
            StateProduct::SNOSProcessing(data) => Some(data.timestamp),
            StateProduct::SNOSVerification(data) => Some(data.verification_timestamp),
            StateProduct::L3ProofCreation(data) => Some(data.proof_timestamp),
            StateProduct::L2ProofCreationProcessing(data) => Some(data.proof_timestamp),
            StateProduct::L2ProofCreationVerification { verification_timestamp, .. } => Some(*verification_timestamp),
            StateProduct::L2DataSubmissionProcessing(data) => Some(data.submission_timestamp),
            StateProduct::L2DataSubmissionVerification { verification_timestamp, .. } => Some(*verification_timestamp),
            StateProduct::L2StateUpdateProcessing(data) => Some(data.update_timestamp),
            StateProduct::L2StateUpdateVerification { verification_timestamp, .. } => Some(*verification_timestamp),
        }
    }

    /// Check if this state product represents a verification state
    pub fn is_verification_state(&self) -> bool {
        matches!(
            self,
            StateProduct::SNOSVerification(_) |
            StateProduct::L2ProofCreationVerification { .. } |
            StateProduct::L2DataSubmissionVerification { .. } |
            StateProduct::L2StateUpdateVerification { .. }
        )
    }
}