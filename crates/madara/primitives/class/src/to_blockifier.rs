use blockifier::execution::contract_class::RunnableCompiledClass;
use starknet_api::contract_class::ContractClass as ApiContractClass;

use crate::{ConvertedClass, LegacyConvertedClass, SierraConvertedClass};

#[cfg(feature = "cairo_native")]
static CACHE: std::sync::LazyLock<
    dashmap::DashMap<
        starknet_types_core::felt::Felt,
        blockifier::execution::native::contract_class::NativeCompiledClassV1,
    >,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = String;

    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => Ok(RunnableCompiledClass::try_from(
                ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi().unwrap()),
            )
            .unwrap()),
            ConvertedClass::Sierra(SierraConvertedClass { class_hash, compiled, info }) => {
                #[cfg(not(feature = "cairo_native"))]
                {
                    let sierra_version = info.contract_class.sierra_version().unwrap();
                    Ok(RunnableCompiledClass::try_from(ApiContractClass::V1((
                        compiled.to_casm().unwrap(),
                        sierra_version,
                    )))
                    .unwrap())
                }

                #[cfg(feature = "cairo_native")]
                {
                    use blockifier::execution::native::contract_class::NativeCompiledClassV1;
                    use cairo_native::executor::AotContractExecutor;
                    use std::path::Path;

                    if let Some(cached) = CACHE.get(class_hash) {
                        return Ok(cached.clone().into());
                    }

                    let sierra_version = info.contract_class.sierra_version().unwrap();

                    let path = Path::new("/usr/share/madara/data/classes").join(format!("{:#x}.so", class_hash));

                    let executor = match AotContractExecutor::from_path(&path) {
                        Ok(Some(executor)) => executor,
                        _ => {
                            println!("compile_to_native class {:#x}", class_hash);
                            println!("path: {:#?}", path);
                            let start = std::time::Instant::now();
                            let executor = info.contract_class.compile_to_native(&path).unwrap();
                            println!("compile_to_native took {:?}", start.elapsed());
                            executor
                        }
                    };

                    let casm = compiled.to_casm().unwrap();
                    let versionned_casm = (casm, sierra_version);
                    let blockifier_compiled_class = versionned_casm.try_into().unwrap();
                    let native_compiled_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);

                    CACHE.insert(*class_hash, native_compiled_class.clone());

                    Ok(native_compiled_class.into())
                }
            }
        }
    }
}
