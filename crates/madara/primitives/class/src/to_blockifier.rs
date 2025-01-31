use blockifier::execution::contract_class::RunnableCompiledClass;
use starknet_api::contract_class::ContractClass as ApiContractClass;

use crate::{ConvertedClass, LegacyConvertedClass, SierraConvertedClass};

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = String;

    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => Ok(RunnableCompiledClass::try_from(
                ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi().unwrap()),
            )
            .unwrap()),
            ConvertedClass::Sierra(SierraConvertedClass { compiled, info, .. }) => {
                let sierra_version = info.contract_class.sierra_version().unwrap();
                Ok(RunnableCompiledClass::try_from(ApiContractClass::V1((compiled.to_casm().unwrap(), sierra_version)))
                    .unwrap())
            }
        }
    }
}
