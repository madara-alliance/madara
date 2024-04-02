#![cfg_attr(not(feature = "std"), no_std)]

#[doc(hidden)]
pub extern crate alloc;

use alloc::vec::Vec;

use blockifier::transaction::objects::TransactionExecutionInfo;
use starknet_core::types::{SimulationFlag, SimulationFlagForEstimateFee as EstimateFeeFlag};

// TODO: This is a placeholder
// https://github.com/starkware-libs/starknet-specs/blob/master/api/starknet_api_openrpc.json#L3919
// The official rpc expect use to return the trace up to the point of failure.
// Figuring out how to get that is a problem for later
#[derive(Debug)]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
pub struct PlaceHolderErrorTypeForFailedStarknetExecution;

pub type TransactionSimulationResult = Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct SimulationFlagForEstimateFee {
    pub skip_validate: bool,
}

pub fn convert_flags(flags: Vec<EstimateFeeFlag>) -> Vec<SimulationFlagForEstimateFee> {
    flags
        .iter()
        .map(|flag| match flag {
            EstimateFeeFlag::SkipValidate => SimulationFlagForEstimateFee { skip_validate: false },
        })
        .collect()
}

impl core::default::Default for SimulationFlagForEstimateFee {
    fn default() -> Self {
        Self { skip_validate: true }
    }
}
