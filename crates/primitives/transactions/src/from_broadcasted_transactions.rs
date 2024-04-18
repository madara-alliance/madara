use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use blockifier::execution::contract_class::{
    ClassInfo, ContractClass, ContractClassV0, ContractClassV0Inner, ContractClassV1,
};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transactions as btx;
use cairo_lang_starknet_classes::casm_contract_class::{
    CasmContractClass, CasmContractEntryPoint, CasmContractEntryPoints, StarknetSierraCompilationError,
};
use cairo_lang_starknet_classes::contract_class::{
    ContractClass as SierraContractClass, ContractEntryPoint, ContractEntryPoints,
};
use cairo_lang_utils::bigint::BigUintAsHex;
use cairo_vm::types::program::Program;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use num_bigint::{BigInt, BigUint, Sign};
use starknet_api::core::{calculate_contract_address, EntryPointSelector};
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointOffset, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{self as stx};
use starknet_core::types::contract::legacy::{
    LegacyContractClass, LegacyEntrypointOffset, RawLegacyEntryPoint, RawLegacyEntryPoints,
};
use starknet_core::types::contract::{CompiledClass, CompiledClassEntrypoint, CompiledClassEntrypointList};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeclareTransactionV1, BroadcastedDeclareTransactionV2,
    BroadcastedDeclareTransactionV3, BroadcastedDeployAccountTransaction, BroadcastedDeployAccountTransactionV1,
    BroadcastedDeployAccountTransactionV3, BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV1,
    BroadcastedInvokeTransactionV3, BroadcastedTransaction, CompressedLegacyContractClass, EntryPointsByType,
    FlattenedSierraClass, LegacyContractEntryPoint, LegacyEntryPointsByType, SierraEntryPoint,
};
use starknet_crypto::FieldElement;
use thiserror::Error;

use crate::compute_hash::ComputeTransactionHash;

#[derive(Debug, Error)]
pub enum BroadcastedTransactionConversionError {
    #[error("Max fee should not be greater than u128::MAX")]
    MaxFeeTooBig,
    #[error("Failed to decompress the program")]
    ProgramDecompressionFailed,
    #[error("Failed to deserialize the program")]
    ProgramDeserializationFailed,
    #[error("Failed compute the hash of the contract class")]
    ClassHashComputationFailed,
    #[error("Failed to convert to CasmContractClass")]
    CasmContractClassConversionFailed,
    #[error("Compiled class hash does not match the class hash")]
    InvalidCompiledClassHash,
    #[error("Failed to compile to Sierra")]
    SierraCompilationFailed,
    #[error("This transaction version is not supported")]
    UnsuportedTransactionVersion,
}

pub trait ToAccountTransaction {
    fn to_account_transaction(&self) -> Result<AccountTransaction, BroadcastedTransactionConversionError>;
}

// TODO: remove clone() and change method with &self
impl ToAccountTransaction for BroadcastedTransaction {
    fn to_account_transaction(&self) -> Result<AccountTransaction, BroadcastedTransactionConversionError> {
        match self {
            BroadcastedTransaction::Invoke(tx) => invoke_to_account_transaction(tx.clone()),
            BroadcastedTransaction::Declare(tx) => declare_to_account_transaction(tx.clone()),
            BroadcastedTransaction::DeployAccount(tx) => deploy_account_to_account_transaction(tx.clone()),
        }
    }
}

fn declare_to_account_transaction(
    value: BroadcastedDeclareTransaction,
) -> Result<AccountTransaction, BroadcastedTransactionConversionError> {
    let user_tx = match value {
        BroadcastedDeclareTransaction::V1(BroadcastedDeclareTransactionV1 {
            sender_address,
            max_fee,
            signature,
            nonce,
            contract_class,
            is_query: _,
        }) => {
            // Create a GzipDecoder to decompress the bytes
            let mut gz = GzDecoder::new(&contract_class.program[..]);

            // Read the decompressed bytes into a Vec<u8>
            let mut decompressed_bytes = Vec::new();
            std::io::Read::read_to_end(&mut gz, &mut decompressed_bytes)
                .map_err(|_| BroadcastedTransactionConversionError::ProgramDecompressionFailed)?;

            let class_hash = {
                let legacy_contract_class = LegacyContractClass {
                    program: serde_json::from_slice(decompressed_bytes.as_slice())
                        .map_err(|_| BroadcastedTransactionConversionError::ProgramDeserializationFailed)?,
                    abi: match contract_class.abi.as_ref() {
                        Some(abi) => Some(abi.iter().cloned().map(|entry| entry.into()).collect::<Vec<_>>()),
                        None => vec![].into(),
                    },
                    entry_points_by_type: to_raw_legacy_entry_points(contract_class.entry_points_by_type.clone()),
                };

                legacy_contract_class
                    .class_hash()
                    .map_err(|_| BroadcastedTransactionConversionError::ClassHashComputationFailed)?
            };

            let blockifier_contract_class = instantiate_blockifier_contract_class(&contract_class, decompressed_bytes)?;

            let declare_tx = stx::DeclareTransaction::V1(stx::DeclareTransactionV0V1 {
                max_fee: stx::Fee(
                    u128::try_from(Felt252Wrapper::from(max_fee))
                        .map_err(|_| BroadcastedTransactionConversionError::MaxFeeTooBig)
                        .unwrap(),
                ),
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                class_hash: Felt252Wrapper::from(class_hash).into(),
                sender_address: Felt252Wrapper::from(sender_address).into(),
            });

            // TODO: defaulted chain id
            let tx_hash = declare_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);
            let class_info = ClassInfo::new(
                &blockifier_contract_class,
                contract_class.clone().program.len(),
                contract_class.abi.clone().unwrap().len(),
            )
            .unwrap();

            let tx = btx::DeclareTransaction::new(declare_tx, tx_hash, class_info).unwrap();

            AccountTransaction::Declare(tx)
        }
        BroadcastedDeclareTransaction::V2(BroadcastedDeclareTransactionV2 {
            sender_address,
            compiled_class_hash,
            max_fee,
            signature,
            nonce,
            contract_class,
            is_query: _,
        }) => {
            let casm_contract_class = flattened_sierra_to_casm_contract_class(&contract_class)
                .map_err(|_| BroadcastedTransactionConversionError::SierraCompilationFailed)?;

            // ensure that the user has sign the correct class hash
            if get_casm_contract_class_hash(&casm_contract_class) != compiled_class_hash {
                return Err(BroadcastedTransactionConversionError::InvalidCompiledClassHash);
            }

            let blockifier_contract_class = ContractClass::V1(
                ContractClassV1::try_from(casm_contract_class)
                    .map_err(|_| BroadcastedTransactionConversionError::CasmContractClassConversionFailed)?,
            );

            let declare_tx = stx::DeclareTransaction::V2(stx::DeclareTransactionV2 {
                max_fee: stx::Fee(
                    u128::try_from(Felt252Wrapper::from(max_fee))
                        .map_err(|_| BroadcastedTransactionConversionError::MaxFeeTooBig)
                        .unwrap(),
                ),
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                class_hash: Felt252Wrapper::from(contract_class.class_hash()).into(),
                sender_address: Felt252Wrapper::from(sender_address).into(),
                compiled_class_hash: Felt252Wrapper::from(compiled_class_hash).into(),
            });

            // TODO: use real chain id
            let tx_hash = declare_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);
            let class_info = ClassInfo::new(
                &blockifier_contract_class,
                contract_class.sierra_program.len(),
                contract_class.abi.len(),
            )
            .unwrap();

            let tx = btx::DeclareTransaction::new(declare_tx, tx_hash, class_info).unwrap();

            AccountTransaction::Declare(tx)
        }
        BroadcastedDeclareTransaction::V3(BroadcastedDeclareTransactionV3 {
            sender_address,
            compiled_class_hash,
            signature,
            nonce,
            contract_class,
            resource_bounds,
            tip,
            paymaster_data,
            account_deployment_data,
            nonce_data_availability_mode,
            fee_data_availability_mode,
            is_query: _,
        }) => {
            let casm_contract_class = flattened_sierra_to_casm_contract_class(&contract_class)
                .map_err(|_| BroadcastedTransactionConversionError::SierraCompilationFailed)?;

            // ensure that the user has sign the correct class hash
            if get_casm_contract_class_hash(&casm_contract_class) != compiled_class_hash {
                return Err(BroadcastedTransactionConversionError::InvalidCompiledClassHash);
            }

            let blockifier_contract_class = ContractClass::V1(
                ContractClassV1::try_from(casm_contract_class)
                    .map_err(|_| BroadcastedTransactionConversionError::CasmContractClassConversionFailed)?,
            );

            let class_hash = contract_class.clone().class_hash();
            let class_hash = Felt252Wrapper::from(class_hash).into();

            let declare_tx = stx::DeclareTransaction::V3(stx::DeclareTransactionV3 {
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                sender_address: Felt252Wrapper::from(sender_address).into(),
                class_hash,
                compiled_class_hash: Felt252Wrapper::from(compiled_class_hash).into(),
                resource_bounds: core_resources_to_api_resources(resource_bounds),
                tip: stx::Tip(tip),
                paymaster_data: stx::PaymasterData(
                    paymaster_data.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                account_deployment_data: stx::AccountDeploymentData(
                    account_deployment_data.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce_data_availability_mode: core_da_to_api_da(nonce_data_availability_mode),
                fee_data_availability_mode: core_da_to_api_da(fee_data_availability_mode),
            });

            // TODO: use real chain id
            let tx_hash = declare_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);
            let class_info = ClassInfo::new(
                &blockifier_contract_class,
                contract_class.sierra_program.len(),
                contract_class.abi.len(),
            )
            .unwrap();

            let tx = btx::DeclareTransaction::new(declare_tx, tx_hash, class_info).unwrap();

            AccountTransaction::Declare(tx)
        }
    };

    Ok(user_tx)
}

fn invoke_to_account_transaction(
    value: BroadcastedInvokeTransaction,
) -> Result<AccountTransaction, BroadcastedTransactionConversionError> {
    let user_tx = match value {
        BroadcastedInvokeTransaction::V1(BroadcastedInvokeTransactionV1 {
            sender_address,
            calldata,
            max_fee,
            signature,
            nonce,
            is_query: _,
            ..
        }) => {
            let invoke_tx = stx::InvokeTransaction::V1(stx::InvokeTransactionV1 {
                max_fee: stx::Fee(
                    u128::try_from(Felt252Wrapper::from(max_fee))
                        .map_err(|_| BroadcastedTransactionConversionError::MaxFeeTooBig)
                        .unwrap(),
                ),
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                sender_address: Felt252Wrapper::from(sender_address).into(),
                calldata: stx::Calldata(
                    calldata.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>().into(),
                ),
            });

            let tx_hash = invoke_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);

            let tx = btx::InvokeTransaction::new(invoke_tx, tx_hash);

            AccountTransaction::Invoke(tx)
        }
        BroadcastedInvokeTransaction::V3(BroadcastedInvokeTransactionV3 {
            sender_address,
            calldata,
            signature,
            nonce,
            resource_bounds,
            tip,
            paymaster_data,
            account_deployment_data,
            nonce_data_availability_mode,
            fee_data_availability_mode,
            is_query: _,
        }) => {
            let invoke_tx = stx::InvokeTransaction::V3(stx::InvokeTransactionV3 {
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                sender_address: Felt252Wrapper::from(sender_address).into(),
                calldata: stx::Calldata(
                    calldata.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>().into(),
                ),
                resource_bounds: core_resources_to_api_resources(resource_bounds),
                tip: stx::Tip(tip),
                paymaster_data: stx::PaymasterData(
                    paymaster_data.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                account_deployment_data: stx::AccountDeploymentData(
                    account_deployment_data.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce_data_availability_mode: core_da_to_api_da(nonce_data_availability_mode),
                fee_data_availability_mode: core_da_to_api_da(fee_data_availability_mode),
            });

            let tx_hash = invoke_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);

            let tx = btx::InvokeTransaction::new(invoke_tx, tx_hash);

            AccountTransaction::Invoke(tx)
        }
    };
    Ok(user_tx)
}

fn deploy_account_to_account_transaction(
    tx: BroadcastedDeployAccountTransaction,
) -> Result<AccountTransaction, BroadcastedTransactionConversionError> {
    let user_tx = match tx {
        BroadcastedDeployAccountTransaction::V1(BroadcastedDeployAccountTransactionV1 {
            max_fee,
            signature,
            nonce,
            contract_address_salt,
            constructor_calldata,
            class_hash,
            is_query: _,
        }) => {
            let deploy_account_tx = stx::DeployAccountTransaction::V1(stx::DeployAccountTransactionV1 {
                max_fee: stx::Fee(
                    u128::try_from(Felt252Wrapper::from(max_fee))
                        .map_err(|_| BroadcastedTransactionConversionError::MaxFeeTooBig)
                        .unwrap(),
                ),
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                contract_address_salt: Felt252Wrapper::from(contract_address_salt).into(),
                constructor_calldata: stx::Calldata(
                    constructor_calldata
                        .iter()
                        .map(|x| Felt252Wrapper::from(*x).into())
                        .collect::<Vec<StarkFelt>>()
                        .into(),
                ),
                class_hash: Felt252Wrapper::from(class_hash).into(),
            });

            let tx_hash = deploy_account_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);

            let contract_address = calculate_contract_address(
                Felt252Wrapper::from(contract_address_salt).into(),
                Felt252Wrapper::from(class_hash).into(),
                &deploy_account_tx.constructor_calldata(),
                Default::default(),
            )
            .unwrap();

            let tx = btx::DeployAccountTransaction::new(deploy_account_tx, tx_hash, contract_address);

            AccountTransaction::DeployAccount(tx)
        }
        BroadcastedDeployAccountTransaction::V3(BroadcastedDeployAccountTransactionV3 {
            signature,
            nonce,
            contract_address_salt,
            constructor_calldata,
            class_hash,
            resource_bounds,
            tip,
            paymaster_data,
            nonce_data_availability_mode,
            fee_data_availability_mode,
            is_query: _,
        }) => {
            let deploy_account_tx = stx::DeployAccountTransaction::V3(stx::DeployAccountTransactionV3 {
                signature: stx::TransactionSignature(
                    signature.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce: Felt252Wrapper::from(nonce).into(),
                contract_address_salt: Felt252Wrapper::from(contract_address_salt).into(),
                constructor_calldata: stx::Calldata(
                    constructor_calldata
                        .iter()
                        .map(|x| Felt252Wrapper::from(*x).into())
                        .collect::<Vec<StarkFelt>>()
                        .into(),
                ),
                class_hash: Felt252Wrapper::from(class_hash).into(),
                resource_bounds: core_resources_to_api_resources(resource_bounds),
                tip: stx::Tip(tip),
                paymaster_data: stx::PaymasterData(
                    paymaster_data.iter().map(|x| Felt252Wrapper::from(*x).into()).collect::<Vec<StarkFelt>>(),
                ),
                nonce_data_availability_mode: core_da_to_api_da(nonce_data_availability_mode),
                fee_data_availability_mode: core_da_to_api_da(fee_data_availability_mode),
            });

            let tx_hash = deploy_account_tx.compute_hash::<PedersenHasher>(Felt252Wrapper::ZERO, false, None);

            let contract_address = calculate_contract_address(
                Felt252Wrapper::from(contract_address_salt).into(),
                Felt252Wrapper::from(class_hash).into(),
                &deploy_account_tx.constructor_calldata(),
                Default::default(),
            )
            .unwrap();

            let tx = btx::DeployAccountTransaction::new(deploy_account_tx, tx_hash, contract_address);

            AccountTransaction::DeployAccount(tx)
        }
    };
    Ok(user_tx)
}

// TODO: is this function needed?
#[allow(dead_code)]
fn cast_vec_of_field_elements(data: Vec<FieldElement>) -> Vec<Felt252Wrapper> {
    // Non-copy but less dangerous than transmute
    // https://doc.rust-lang.org/std/mem/fn.transmute.html#alternatives

    // Unsafe code but all invariants are checked:

    // 1. ptr must have been allocated using the global allocator -> data is allocated with the Global
    //    allocator.
    // 2. T needs to have the same alignment as what ptr was allocated with -> Felt252Wrapper uses
    //    transparent representation of the inner type.
    // 3. The allocated size in bytes needs to be the same as the pointer -> As FieldElement and
    //    Felt252Wrapper have the same size, and capacity is taken directly from the data Vector, we
    //    will have the same allocated byte size.
    // 4. Length needs to be less than or equal to capacity -> data.len() is always less than or equal
    //    to data.capacity()
    // 5. The first length values must be properly initialized values of type T -> ok since we use data
    //    which was correctly allocated
    // 6. capacity needs to be the capacity that the pointer was allocated with -> data.as_mut_ptr()
    //    returns a pointer to memory having at least capacity initialized memory
    // 7. The allocated size in bytes must be no larger than isize::MAX -> data.capacity() will never be
    //    bigger than isize::MAX (https://doc.rust-lang.org/std/vec/struct.Vec.html#panics-7)
    let mut data = core::mem::ManuallyDrop::new(data);
    unsafe { alloc::vec::Vec::from_raw_parts(data.as_mut_ptr() as *mut Felt252Wrapper, data.len(), data.capacity()) }
}

fn instantiate_blockifier_contract_class(
    contract_class: &Arc<CompressedLegacyContractClass>,
    program_decompressed_bytes: Vec<u8>,
) -> Result<ContractClass, BroadcastedTransactionConversionError> {
    // Deserialize it then
    let program: Program = Program::from_bytes(&program_decompressed_bytes, None)
        .map_err(|_| BroadcastedTransactionConversionError::ProgramDeserializationFailed)?;

    let mut entry_points_by_type = <IndexMap<EntryPointType, Vec<EntryPoint>>>::new();
    entry_points_by_type.insert(
        EntryPointType::Constructor,
        contract_class
            .entry_points_by_type
            .constructor
            .iter()
            .map(|entry_point| -> EntryPoint {
                EntryPoint {
                    selector: EntryPointSelector(StarkFelt(entry_point.selector.to_bytes_be())),
                    offset: EntryPointOffset(entry_point.offset),
                }
            })
            .collect::<Vec<EntryPoint>>(),
    );
    entry_points_by_type.insert(
        EntryPointType::External,
        contract_class
            .entry_points_by_type
            .external
            .iter()
            .map(|entry_point| -> EntryPoint {
                EntryPoint {
                    selector: EntryPointSelector(StarkFelt(entry_point.selector.to_bytes_be())),
                    offset: EntryPointOffset(entry_point.offset),
                }
            })
            .collect::<Vec<EntryPoint>>(),
    );
    entry_points_by_type.insert(
        EntryPointType::L1Handler,
        contract_class
            .entry_points_by_type
            .l1_handler
            .iter()
            .map(|entry_point| -> EntryPoint {
                EntryPoint {
                    selector: EntryPointSelector(StarkFelt(entry_point.selector.to_bytes_be())),
                    offset: EntryPointOffset(entry_point.offset),
                }
            })
            .collect::<Vec<EntryPoint>>(),
    );

    let contract_class =
        ContractClass::V0(ContractClassV0(Arc::new(ContractClassV0Inner { program, entry_points_by_type })));

    Ok(contract_class)
}

fn to_raw_legacy_entry_point(entry_point: LegacyContractEntryPoint) -> RawLegacyEntryPoint {
    RawLegacyEntryPoint { offset: LegacyEntrypointOffset::U64AsInt(entry_point.offset), selector: entry_point.selector }
}

fn to_raw_legacy_entry_points(entry_points: LegacyEntryPointsByType) -> RawLegacyEntryPoints {
    RawLegacyEntryPoints {
        constructor: entry_points.constructor.into_iter().map(to_raw_legacy_entry_point).collect(),
        external: entry_points.external.into_iter().map(to_raw_legacy_entry_point).collect(),
        l1_handler: entry_points.l1_handler.into_iter().map(to_raw_legacy_entry_point).collect(),
    }
}

/// Converts a [FlattenedSierraClass] to a [CasmContractClass]
pub fn flattened_sierra_to_casm_contract_class(
    flattened_sierra: &Arc<FlattenedSierraClass>,
) -> Result<CasmContractClass, StarknetSierraCompilationError> {
    let sierra_contract_class = SierraContractClass {
        sierra_program: flattened_sierra.sierra_program.iter().map(field_element_to_big_uint_as_hex).collect(),
        sierra_program_debug_info: None, // TODO: implement a correct sierra program debug info conversion
        contract_class_version: flattened_sierra.contract_class_version.clone(),
        entry_points_by_type: entry_points_by_type_to_contract_entry_points(
            flattened_sierra.entry_points_by_type.clone(),
        ),
        abi: None, // TODO: implement a correct abi conversion
    };
    let casm_contract_class = CasmContractClass::from_contract_class(sierra_contract_class, false, usize::MAX)?;

    Ok(casm_contract_class)
}

/// Converts a [FieldElement] to a [BigUint]
fn field_element_to_big_uint(value: &FieldElement) -> BigUint {
    BigInt::from_bytes_be(Sign::Plus, &value.to_bytes_be()).to_biguint().unwrap()
}

/// Converts a [FieldElement] to a [BigUintAsHex]
fn field_element_to_big_uint_as_hex(value: &FieldElement) -> BigUintAsHex {
    BigUintAsHex { value: field_element_to_big_uint(value) }
}

/// Converts a [EntryPointsByType] to a [ContractEntryPoints]
fn entry_points_by_type_to_contract_entry_points(value: EntryPointsByType) -> ContractEntryPoints {
    fn sierra_entry_point_to_contract_entry_point(value: SierraEntryPoint) -> ContractEntryPoint {
        ContractEntryPoint {
            function_idx: value.function_idx.try_into().unwrap(),
            selector: field_element_to_big_uint(&value.selector),
        }
    }
    ContractEntryPoints {
        constructor: value.constructor.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        external: value.external.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
        l1_handler: value.l1_handler.iter().map(|x| sierra_entry_point_to_contract_entry_point(x.clone())).collect(),
    }
}

// Utils to convert Casm contract class to Compiled class
pub fn get_casm_contract_class_hash(casm_contract_class: &CasmContractClass) -> FieldElement {
    let compiled_class = casm_contract_class_to_compiled_class(casm_contract_class);
    compiled_class.class_hash().unwrap()
}

/// Converts a [CasmContractClass] to a [CompiledClass]
pub fn casm_contract_class_to_compiled_class(casm_contract_class: &CasmContractClass) -> CompiledClass {
    CompiledClass {
        prime: casm_contract_class.prime.to_string(),
        compiler_version: casm_contract_class.compiler_version.clone(),
        bytecode: casm_contract_class.bytecode.iter().map(|x| biguint_to_field_element(&x.value)).collect(),
        entry_points_by_type: casm_entry_points_to_compiled_entry_points(&casm_contract_class.entry_points_by_type),
        hints: vec![],                    // not needed to get class hash so ignoring this
        pythonic_hints: None,             // not needed to get class hash so ignoring this
        bytecode_segment_lengths: vec![], // TODO: implement this
    }
}

/// Converts a [BigUint] to a [FieldElement]
fn biguint_to_field_element(value: &BigUint) -> FieldElement {
    let bytes = value.to_bytes_be();
    FieldElement::from_byte_slice_be(bytes.as_slice()).unwrap()
}

/// Converts a [CasmContractEntryPoints] to a [CompiledClassEntrypointList]
fn casm_entry_points_to_compiled_entry_points(value: &CasmContractEntryPoints) -> CompiledClassEntrypointList {
    CompiledClassEntrypointList {
        external: value.external.iter().map(casm_entry_point_to_compiled_entry_point).collect(),
        l1_handler: value.l1_handler.iter().map(casm_entry_point_to_compiled_entry_point).collect(),
        constructor: value.constructor.iter().map(casm_entry_point_to_compiled_entry_point).collect(),
    }
}

/// Converts a [CasmContractEntryPoint] to a [CompiledClassEntrypoint]
fn casm_entry_point_to_compiled_entry_point(value: &CasmContractEntryPoint) -> CompiledClassEntrypoint {
    CompiledClassEntrypoint {
        selector: biguint_to_field_element(&value.selector),
        offset: value.offset.try_into().unwrap(),
        builtins: value.builtins.clone(),
    }
}

pub fn core_da_to_api_da(
    da: starknet_core::types::DataAvailabilityMode,
) -> starknet_api::data_availability::DataAvailabilityMode {
    match da {
        starknet_core::types::DataAvailabilityMode::L1 => starknet_api::data_availability::DataAvailabilityMode::L1,
        starknet_core::types::DataAvailabilityMode::L2 => starknet_api::data_availability::DataAvailabilityMode::L2,
    }
}

// TODO: check here for L1 gas instead of L2 gas
pub fn core_resources_to_api_resources(
    core_resources: starknet_core::types::ResourceBoundsMapping,
) -> stx::ResourceBoundsMapping {
    let mut api_resources_map = BTreeMap::new();

    // Convert L1 gas ResourceBounds
    api_resources_map.insert(
        stx::Resource::L1Gas, // Assuming you have an enum Resource with L1Gas and L2Gas variants
        stx::ResourceBounds {
            max_amount: core_resources.l1_gas.max_amount,
            max_price_per_unit: core_resources.l1_gas.max_price_per_unit,
        },
    );

    // Convert L2 gas ResourceBounds
    api_resources_map.insert(
        stx::Resource::L2Gas,
        stx::ResourceBounds {
            max_amount: core_resources.l2_gas.max_amount,
            max_price_per_unit: core_resources.l2_gas.max_price_per_unit,
        },
    );

    stx::ResourceBoundsMapping(api_resources_map)
}
