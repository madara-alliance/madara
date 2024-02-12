//! Starknet block primitives.
#![cfg_attr(not(feature = "std"), no_std)]

#[doc(hidden)]
pub extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "parity-scale-codec")]
use parity_scale_codec::{Decode, Encode};

pub mod class;

// TODO: move this somewhere more sensible? Would be a good idea to decouple
// publicly available storage data from wrapper classes
#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum ContractAbi {
    Sierra(String),
    Cairo(Option<Vec<AbiEntryWrapper>>),
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum AbiEntryWrapper {
    Function(AbiFunctionEntryWrapper),
    Event(AbiEventEntryWrapper),
    Struct(AbiStructEntryWrapper),
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct AbiFunctionEntryWrapper {
    // Function abi type
    pub r#type: AbiFunctionTypeWrapper,
    /// The function name
    pub name: String,
    /// Typed parameter
    pub inputs: Vec<AbiTypedParameterWrapper>,
    /// Typed parameter
    pub outputs: Vec<AbiTypedParameterWrapper>,
    /// Function state mutability
    pub state_mutability: Option<AbiFunctionStateMutabilityWrapper>,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct AbiEventEntryWrapper {
    /// Event abi type
    pub r#type: AbiEventTypeWrapper,
    /// The event name
    pub name: String,
    /// Typed parameter
    pub keys: Vec<AbiTypedParameterWrapper>,
    /// Typed parameter
    pub data: Vec<AbiTypedParameterWrapper>,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct AbiStructEntryWrapper {
    pub r#type: AbiStructTypeWrapper,
    pub name: String,
    pub size: u64,
    pub members: Vec<AbiStructMemberWrapper>,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct AbiStructMemberWrapper {
    /// The parameter's name
    pub name: String,
    /// The parameter's type
    pub r#type: String,
    /// Offset of this property within the struct
    pub offset: u64,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum AbiFunctionTypeWrapper {
    Function,
    L1handler,
    Constructor,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum AbiEventTypeWrapper {
    Event,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum AbiStructTypeWrapper {
    Struct,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum AbiFunctionStateMutabilityWrapper {
    View,
}

#[derive(Debug)]
#[cfg_attr(feature = "parity-scale-codec", derive(Encode, Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub struct AbiTypedParameterWrapper {
    pub name: String,
    pub r#type: String,
}
