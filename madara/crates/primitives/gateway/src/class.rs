use serde::{Deserialize, Serialize};
use starknet_core::types::contract::legacy::{LegacyProgram, RawLegacyAbiEntry, RawLegacyEntryPoints};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "deny_unknown_fields", serde(deny_unknown_fields))]
pub struct LegacyContractClass {
    /// Contract ABI.
    #[serde(default = "Vec::new")]
    pub abi: Vec<RawLegacyAbiEntry>,
    /// Contract entrypoints.
    pub entry_points_by_type: RawLegacyEntryPoints,
    /// The Cairo program of the contract containing the actual bytecode.
    pub program: LegacyProgram,
}

impl From<mp_class::LegacyContractClass> for LegacyContractClass {
    fn from(value: mp_class::LegacyContractClass) -> Self {
        Self {
            abi: value.abi.unwrap_or_default(),
            entry_points_by_type: value.entry_points_by_type,
            program: value.program,
        }
    }
}
impl From<LegacyContractClass> for mp_class::LegacyContractClass {
    fn from(value: LegacyContractClass) -> Self {
        Self {
            abi: if value.abi.is_empty() { None } else { Some(value.abi) },
            entry_points_by_type: value.entry_points_by_type,
            program: value.program,
        }
    }
}
