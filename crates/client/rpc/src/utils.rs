use std::collections::HashMap;
use std::io::Write;
use std::num::NonZeroU128;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use blockifier::execution::call_info::CallInfo;
use blockifier::execution::contract_class::ContractClass as BlockifierContractClass;
use cairo_lang_starknet_classes::casm_contract_class::{
    CasmContractClass, CasmContractEntryPoint, CasmContractEntryPoints,
};
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mp_block::DeoxysBlock;
use mp_digest_log::find_starknet_block;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_types::block::{DBlockT, DHashT};
use num_bigint::BigUint;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sp_api::{BlockT, HeaderT, ProvideRuntimeApi};
use sp_blockchain::HeaderBackend;
use sp_runtime::DispatchError;
use starknet_api::deprecated_contract_class::{EntryPoint, EntryPointType};
use starknet_api::hash::StarkFelt;
use starknet_api::state::ThinStateDiff;
use starknet_api::transaction as stx;
use starknet_core::types::contract::{CompiledClass, CompiledClassEntrypoint, CompiledClassEntrypointList};
use starknet_core::types::{
    BlockStatus, CompressedLegacyContractClass, ComputationResources, ContractClass, ContractStorageDiffItem,
    DataAvailabilityResources, DataResources, DeclaredClassItem, DeployedContractItem, EntryPointsByType, Event,
    ExecutionResources, FieldElement, FlattenedSierraClass, FromByteArrayError, L1DataAvailabilityMode,
    LegacyContractEntryPoint, LegacyEntryPointsByType, MsgToL1, NonceUpdate, ReplacedClassItem, ResourcePrice,
    StateDiff, StorageEntry,
};

use crate::errors::StarknetRpcApiError;
use crate::Felt;

pub fn extract_events_from_call_info(call_info: &CallInfo) -> Vec<Event> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .events
        .iter()
        .map(|ordered_event| Event {
            from_address: FieldElement::from_byte_slice_be(address.0.0.bytes()).unwrap(),
            keys: ordered_event
                .event
                .keys
                .iter()
                .map(|key| FieldElement::from_byte_slice_be(key.0.bytes()).unwrap())
                .collect(),
            data: ordered_event
                .event
                .data
                .0
                .iter()
                .map(|data_item| FieldElement::from_byte_slice_be(data_item.bytes()).unwrap())
                .collect(),
        })
        .collect();

    let inner_events: Vec<_> = call_info.inner_calls.iter().flat_map(extract_events_from_call_info).collect();

    events.into_iter().chain(inner_events).collect()
}

pub fn extract_messages_from_call_info(call_info: &CallInfo) -> Vec<MsgToL1> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .map(|msg| MsgToL1 {
            from_address: FieldElement::from_byte_slice_be(address.0.0.bytes()).unwrap(),
            to_address: FieldElement::from_byte_slice_be(msg.message.to_address.0.to_fixed_bytes().as_slice()).unwrap(),
            payload: msg
                .message
                .payload
                .0
                .iter()
                .map(|data_item| FieldElement::from_byte_slice_be(data_item.bytes()).unwrap())
                .collect(),
        })
        .collect();

    let inner_messages: Vec<_> = call_info.inner_calls.iter().flat_map(extract_messages_from_call_info).collect();

    events.into_iter().chain(inner_messages).collect()
}

pub fn blockifier_call_info_to_starknet_resources(callinfo: &CallInfo) -> ExecutionResources {
    let vm_resources = &callinfo.resources;

    let steps = vm_resources.n_steps as u64;
    let memory_holes = match vm_resources.n_memory_holes as u64 {
        0 => None,
        n => Some(n),
    };

    let builtin_instance = &vm_resources.builtin_instance_counter;

    let range_check_builtin_applications = builtin_instance.get("range_check_builtin").map(|&value| value as u64);
    let pedersen_builtin_applications = builtin_instance.get("pedersen_builtin").map(|&value| value as u64);
    let poseidon_builtin_applications = builtin_instance.get("poseidon_builtin").map(|&value| value as u64);
    let ec_op_builtin_applications = builtin_instance.get("ec_op_builtin").map(|&value| value as u64);
    let ecdsa_builtin_applications = builtin_instance.get("ecdsa_builtin").map(|&value| value as u64);
    let bitwise_builtin_applications = builtin_instance.get("bitwise_builtin").map(|&value| value as u64);
    let keccak_builtin_applications = builtin_instance.get("keccak_builtin").map(|&value| value as u64);
    let segment_arena_builtin = builtin_instance.get("segment_arena_builtin").map(|&value| value as u64);

    ExecutionResources {
        computation_resources: ComputationResources {
            steps,
            memory_holes,
            range_check_builtin_applications,
            pedersen_builtin_applications,
            poseidon_builtin_applications,
            ec_op_builtin_applications,
            ecdsa_builtin_applications,
            bitwise_builtin_applications,
            keccak_builtin_applications,
            segment_arena_builtin,
        },
        // TODO: add data resources when blockifier supports it
        data_resources: DataResources { data_availability: DataAvailabilityResources { l1_gas: 0, l1_data_gas: 0 } },
    }
}

pub(crate) fn tx_hash_retrieve(tx_hashes: Vec<StarkFelt>) -> Vec<FieldElement> {
    let mut v = Vec::with_capacity(tx_hashes.len());
    for tx_hash in tx_hashes {
        v.push(FieldElement::from(Felt252Wrapper::from(tx_hash)));
    }
    v
}

pub(crate) fn tx_hash_compute<H>(block: &DeoxysBlock, chain_id: Felt) -> Vec<FieldElement>
where
    H: HasherT + Send + Sync + 'static,
{
    block
        .transactions_hashes::<H>(chain_id.0.into(), Some(block.header().block_number))
        .map(|tx_hash| FieldElement::from(Felt252Wrapper::from(tx_hash)))
        .collect()
}

pub(crate) fn tx_conv(
    txs: &[stx::Transaction],
    tx_hashes: Vec<FieldElement>,
) -> Vec<starknet_core::types::Transaction> {
    txs.iter().zip(tx_hashes).map(|(tx, hash)| to_starknet_core_tx(tx.clone(), hash)).collect()
}

pub(crate) fn status(block_number: u64) -> BlockStatus {
    if block_number <= ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    }
}

pub(crate) fn parent_hash(block: &DeoxysBlock) -> FieldElement {
    Felt252Wrapper::from(block.header().parent_block_hash).into()
}

pub(crate) fn new_root(block: &DeoxysBlock) -> FieldElement {
    Felt252Wrapper::from(block.header().global_state_root).into()
}

pub(crate) fn timestamp(block: &DeoxysBlock) -> u64 {
    block.header().block_timestamp
}

pub(crate) fn sequencer_address(block: &DeoxysBlock) -> FieldElement {
    Felt252Wrapper::from(block.header().sequencer_address).into()
}

pub(crate) fn l1_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    // 1 is a special value that means 0 because the gas price is stored as a NonZeroU128
    fn non_zeo_u128_to_field_element(value: NonZeroU128) -> FieldElement {
        match value.get() {
            1 => FieldElement::ZERO,
            x => FieldElement::from(x),
        }
    }

    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: non_zeo_u128_to_field_element(resource_price.strk_l1_gas_price),
            price_in_wei: non_zeo_u128_to_field_element(resource_price.eth_l1_gas_price),
        },
        None => ResourcePrice { price_in_fri: FieldElement::ZERO, price_in_wei: FieldElement::ZERO },
    }
}

pub(crate) fn l1_data_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: resource_price.strk_l1_data_gas_price.get().into(),
            price_in_wei: resource_price.eth_l1_data_gas_price.get().into(),
        },
        None => ResourcePrice { price_in_fri: FieldElement::ONE, price_in_wei: FieldElement::ONE },
    }
}

pub(crate) fn l1_da_mode(block: &DeoxysBlock) -> L1DataAvailabilityMode {
    let l1_da_mode = block.header().l1_da_mode;
    match l1_da_mode {
        starknet_api::data_availability::L1DataAvailabilityMode::Calldata => L1DataAvailabilityMode::Calldata,
        starknet_api::data_availability::L1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
    }
}

pub(crate) fn starknet_version(block: &DeoxysBlock) -> String {
    block.header().protocol_version.from_utf8().expect("starknet version should be a valid utf8 string")
}

/// Returns a [`ContractClass`] from a [`BlockifierContractClass`]
pub fn to_rpc_contract_class(contract_class: BlockifierContractClass) -> Result<ContractClass> {
    match contract_class {
        BlockifierContractClass::V0(contract_class) => {
            let entry_points_by_type: HashMap<_, _> = contract_class.entry_points_by_type.clone().into_iter().collect();
            let entry_points_by_type = to_legacy_entry_points_by_type(&entry_points_by_type)?;
            let compressed_program = compress(&contract_class.program.serialize()?)?;
            Ok(ContractClass::Legacy(CompressedLegacyContractClass {
                program: compressed_program,
                entry_points_by_type,
                // FIXME 723
                abi: None,
            }))
        }
        BlockifierContractClass::V1(_contract_class) => Ok(ContractClass::Sierra(FlattenedSierraClass {
            sierra_program: vec![], // FIXME: https://github.com/keep-starknet-strange/madara/issues/775
            contract_class_version: option_env!("COMPILER_VERSION").unwrap_or("0.11.2").into(),
            entry_points_by_type: EntryPointsByType { constructor: vec![], external: vec![], l1_handler: vec![] }, /* TODO: add entry_points_by_type */
            abi: String::from("{}"), // FIXME: https://github.com/keep-starknet-strange/madara/issues/790
        })),
    }
}

/// Returns a [`StateDiff`] from a [`ThinStateDiff`]
pub fn to_rpc_state_diff(thin_state_diff: ThinStateDiff) -> StateDiff {
    let nonces = thin_state_diff
        .nonces
        .iter()
        .map(|x| NonceUpdate {
            contract_address: Felt252Wrapper::from(x.0.0.0).into(),
            nonce: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let storage_diffs = thin_state_diff
        .storage_diffs
        .iter()
        .map(|x| ContractStorageDiffItem {
            address: Felt252Wrapper::from(x.0.0.0).into(),
            storage_entries: x
                .1
                .iter()
                .map(|y| StorageEntry {
                    key: Felt252Wrapper::from(y.0.0.0).into(),
                    value: Felt252Wrapper::from(*y.1).into(),
                })
                .collect(),
        })
        .collect();

    let deprecated_declared_classes =
        thin_state_diff.deprecated_declared_classes.iter().map(|x| Felt252Wrapper::from(x.0).into()).collect();

    let declared_classes = thin_state_diff
        .declared_classes
        .iter()
        .map(|x| DeclaredClassItem {
            class_hash: Felt252Wrapper::from(x.0.0).into(),
            compiled_class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let deployed_contracts = thin_state_diff
        .deployed_contracts
        .iter()
        .map(|x| DeployedContractItem {
            address: Felt252Wrapper::from(x.0.0.0).into(),
            class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    let replaced_classes = thin_state_diff
        .replaced_classes
        .iter()
        .map(|x| ReplacedClassItem {
            contract_address: Felt252Wrapper::from(x.0.0.0).into(),
            class_hash: Felt252Wrapper::from(x.1.0).into(),
        })
        .collect();

    StateDiff {
        nonces,
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts,
        replaced_classes,
    }
}

/// Returns a compressed vector of bytes
fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut gzip_encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    // 2023-08-22: JSON serialization is already done in Blockifier
    // https://github.com/keep-starknet-strange/blockifier/blob/no_std-support-7578442/crates/blockifier/src/execution/contract_class.rs#L129
    // https://github.com/keep-starknet-strange/blockifier/blob/no_std-support-7578442/crates/blockifier/src/execution/contract_class.rs#L389
    // serde_json::to_writer(&mut gzip_encoder, data)?;
    gzip_encoder.write_all(data)?;
    Ok(gzip_encoder.finish()?)
}

/// Returns a [Result<LegacyEntryPointsByType>] (starknet-rs type) from a [HashMap<EntryPointType,
/// Vec<EntryPoint>>]
fn to_legacy_entry_points_by_type(
    entries: &HashMap<EntryPointType, Vec<EntryPoint>>,
) -> Result<LegacyEntryPointsByType> {
    fn collect_entry_points(
        entries: &HashMap<EntryPointType, Vec<EntryPoint>>,
        entry_point_type: EntryPointType,
    ) -> Result<Vec<LegacyContractEntryPoint>> {
        Ok(entries
            .get(&entry_point_type)
            .ok_or(anyhow!("Missing {:?} entry point", entry_point_type))?
            .iter()
            .map(|e| to_legacy_entry_point(e.clone()))
            .collect::<Result<Vec<LegacyContractEntryPoint>, FromByteArrayError>>()?)
    }

    let constructor = collect_entry_points(entries, EntryPointType::Constructor)?;
    let external = collect_entry_points(entries, EntryPointType::External)?;
    let l1_handler = collect_entry_points(entries, EntryPointType::L1Handler)?;

    Ok(LegacyEntryPointsByType { constructor, external, l1_handler })
}

/// Returns a [LegacyContractEntryPoint] (starknet-rs) from a [EntryPoint] (starknet-api)
fn to_legacy_entry_point(entry_point: EntryPoint) -> Result<LegacyContractEntryPoint, FromByteArrayError> {
    let selector = FieldElement::from_bytes_be(&entry_point.selector.0.0)?;
    let offset = entry_point.offset.0;
    Ok(LegacyContractEntryPoint { selector, offset })
}

/// Returns the current Starknet block from the block header's digest
pub fn get_block_by_block_hash<B, C>(client: &C, block_hash: <B as BlockT>::Hash) -> Result<DeoxysBlock>
where
    B: BlockT,
    C: HeaderBackend<B>,
{
    let header =
        client.header(block_hash).ok().flatten().ok_or_else(|| anyhow::Error::msg("Failed to retrieve header"))?;
    let digest = header.digest();
    let block = find_starknet_block(digest)?;
    Ok(block)
}

// Utils to convert Casm contract class to Compiled class
pub fn get_casm_cotract_class_hash(casm_contract_class: &CasmContractClass) -> FieldElement {
    let compiled_class = casm_contract_class_to_compiled_class(casm_contract_class);
    compiled_class.class_hash().unwrap()
}

/// Converts a [CasmContractClass] to a [CompiledClass]
fn casm_contract_class_to_compiled_class(casm_contract_class: &CasmContractClass) -> CompiledClass {
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

/// Converts a [BigUint] to a [FieldElement]
fn biguint_to_field_element(value: &BigUint) -> FieldElement {
    let bytes = value.to_bytes_be();
    FieldElement::from_byte_slice_be(bytes.as_slice()).unwrap()
}

pub fn convert_error<C, T>(
    client: Arc<C>,
    best_block_hash: DHashT,
    call_result: Result<T, DispatchError>,
) -> Result<T, StarknetRpcApiError>
where
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
{
    match call_result {
        Ok(val) => Ok(val),
        Err(e) => match client.runtime_api().convert_error(best_block_hash, e) {
            Ok(starknet_error) => Err(starknet_error.into()),
            Err(_) => Err(StarknetRpcApiError::InternalServerError),
        },
    }
}
