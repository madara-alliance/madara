use starknet_core::types::SimulationFlag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationFlags {
    pub validate: bool,
    pub charge_fee: bool,
}

impl From<Vec<SimulationFlag>> for SimulationFlags {
    fn from(flags: Vec<SimulationFlag>) -> Self {
        let mut flags_out = Self::default();

        for flag in flags {
            match flag {
                SimulationFlag::SkipValidate => flags_out.validate = false,
                SimulationFlag::SkipFeeCharge => flags_out.charge_fee = false,
            }
            if !flags_out.validate && !flags_out.charge_fee {
                break;
            }
        }

        flags_out
    }
}

impl core::default::Default for SimulationFlags {
    fn default() -> Self {
        Self { validate: true, charge_fee: true }
    }
}
