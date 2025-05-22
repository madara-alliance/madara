use orchestrator_da_client_interface::DaVerificationStatus;
use orchestrator_settlement_client_interface::SettlementVerificationStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobVerificationStatus {
    #[allow(dead_code)]
    Pending,
    #[allow(dead_code)]
    Verified,
    #[allow(dead_code)]
    Rejected(String),
}

impl From<DaVerificationStatus> for JobVerificationStatus {
    fn from(status: DaVerificationStatus) -> Self {
        match status {
            DaVerificationStatus::Pending => JobVerificationStatus::Pending,
            DaVerificationStatus::Verified => JobVerificationStatus::Verified,
            DaVerificationStatus::Rejected(e) => JobVerificationStatus::Rejected(e),
        }
    }
}

impl From<SettlementVerificationStatus> for JobVerificationStatus {
    fn from(status: SettlementVerificationStatus) -> Self {
        match status {
            SettlementVerificationStatus::Pending => JobVerificationStatus::Pending,
            SettlementVerificationStatus::Verified => JobVerificationStatus::Verified,
            SettlementVerificationStatus::Rejected(e) => JobVerificationStatus::Rejected(e),
        }
    }
}
