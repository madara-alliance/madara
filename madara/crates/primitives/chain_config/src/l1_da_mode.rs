use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum L1DataAvailabilityMode {
    #[serde(alias = "Calldata")]
    Calldata,
    #[serde(alias = "Blob")]
    #[default]
    Blob,
}

impl From<L1DataAvailabilityMode> for mp_rpc::L1DaMode {
    fn from(value: L1DataAvailabilityMode) -> Self {
        match value {
            L1DataAvailabilityMode::Calldata => Self::Calldata,
            L1DataAvailabilityMode::Blob => Self::Blob,
        }
    }
}
