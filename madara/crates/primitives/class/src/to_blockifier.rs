use std::str::FromStr;
use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use serde::de::Error as _;
use starknet_api::contract_class::{ContractClass as ApiContractClass, SierraVersion};
use starknet_types_core::felt::Felt;
use crate::{ConvertedClass, LegacyConvertedClass, SierraConvertedClass};

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = ProgramError;

    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => {
                RunnableCompiledClass::try_from(ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi()?))
            }
            ConvertedClass::Sierra(SierraConvertedClass { compiled, info, .. }) => {
                let mut sierra_version = info.contract_class.sierra_version().map_err(|_| {
                    ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                })?;
                
                // let myhash = Felt::from_str("0x65b23055536eea6ee47143c5af666fe8e6504def015be54ea225ee11a39da45").unwrap();
                // if myhash == info.compiled_class_hash {
                //     sierra_version = SierraVersion::new(
                //         1,7,0
                //     )
                // }
                
                println!("DEBUG Sierra version: {:?}", sierra_version);
                println!("DEBUG Sierra version: {:?}", info.compiled_class_hash);
                RunnableCompiledClass::try_from(ApiContractClass::V1((compiled.as_ref().try_into()?, sierra_version)))
            }
        }
    }
}
