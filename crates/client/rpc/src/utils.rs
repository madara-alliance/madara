use blockifier::execution::entry_point::CallInfo;
pub use mc_rpc_core::{Felt, StarknetReadRpcApiServer, StarknetTraceRpcApiServer, StarknetWriteRpcApiServer};
use mc_sync::l1::ETHEREUM_STATE_UPDATE;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockStatus, Event, ExecutionResources, FieldElement, MsgToL1, ResourcePrice, Transaction};

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
    let vm_ressources = &callinfo.vm_resources;

    let steps = vm_ressources.n_steps as u64;
    let memory_holes = match vm_ressources.n_memory_holes as u64 {
        0 => None,
        n => Some(n),
    };

    let builtin_insstance = &vm_ressources.builtin_instance_counter;

    let range_check_builtin_applications = *builtin_insstance.get("range_check_builtin").unwrap_or(&0) as u64;
    let pedersen_builtin_applications = *builtin_insstance.get("pedersen_builtin").unwrap_or(&0) as u64;
    let poseidon_builtin_applications = *builtin_insstance.get("poseidon_builtin").unwrap_or(&0) as u64;
    let ec_op_builtin_applications = *builtin_insstance.get("ec_op_builtin").unwrap_or(&0) as u64;
    let ecdsa_builtin_applications = *builtin_insstance.get("ecdsa_builtin").unwrap_or(&0) as u64;
    let bitwise_builtin_applications = *builtin_insstance.get("bitwise_builtin").unwrap_or(&0) as u64;
    let keccak_builtin_applications = *builtin_insstance.get("keccak_builtin").unwrap_or(&0) as u64;

    ExecutionResources {
        steps,
        memory_holes,
        range_check_builtin_applications,
        pedersen_builtin_applications,
        poseidon_builtin_applications,
        ec_op_builtin_applications,
        ecdsa_builtin_applications,
        bitwise_builtin_applications,
        keccak_builtin_applications,
    }
}

#[allow(dead_code)]
pub fn blockifier_to_starknet_rs_ordered_events(
    ordered_events: &[blockifier::execution::entry_point::OrderedEvent],
) -> Vec<starknet_core::types::OrderedEvent> {
    ordered_events
        .iter()
        .map(|event| starknet_core::types::OrderedEvent {
            order: event.order as u64, // Convert usize to u64
            keys: event.event.keys.iter().map(|key| FieldElement::from_byte_slice_be(key.0.bytes()).unwrap()).collect(),
            data: event
                .event
                .data
                .0
                .iter()
                .map(|data_item| FieldElement::from_byte_slice_be(data_item.bytes()).unwrap())
                .collect(),
        })
        .collect()
}

pub(crate) fn tx_hash_retrieve(tx_hashes: Vec<StarkFelt>) -> Vec<FieldElement> {
    let mut v = Vec::with_capacity(tx_hashes.len());
    for tx_hash in tx_hashes {
        v.push(FieldElement::from(Felt252Wrapper::from(tx_hash)));
    }
    v
}

pub(crate) fn tx_hash_compute<H>(block: &mp_block::Block, chain_id: Felt) -> Vec<FieldElement>
where
    H: HasherT + Send + Sync + 'static,
{
    block
        .transactions_hashes::<H>(chain_id.0.into(), Some(block.header().block_number))
        .map(FieldElement::from)
        .collect()
}

pub(crate) fn tx_conv(txs: &[mp_transactions::Transaction], tx_hashes: Vec<FieldElement>) -> Vec<Transaction> {
    txs.iter().zip(tx_hashes).map(|(tx, hash)| to_starknet_core_tx(tx.clone(), hash)).collect()
}

pub(crate) fn status(block_number: u64) -> BlockStatus {
    if block_number <= ETHEREUM_STATE_UPDATE.read().unwrap().block_number {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    }
}

pub(crate) fn parent_hash(block: &mp_block::Block) -> FieldElement {
    Felt252Wrapper::from(block.header().parent_block_hash).into()
}

pub(crate) fn new_root(block: &mp_block::Block) -> FieldElement {
    Felt252Wrapper::from(block.header().global_state_root).into()
}

pub(crate) fn timestamp(block: &mp_block::Block) -> u64 {
    block.header().block_timestamp
}

pub(crate) fn sequencer_address(block: &mp_block::Block) -> FieldElement {
    Felt252Wrapper::from(block.header().sequencer_address).into()
}

pub(crate) fn l1_gas_price(block: &mp_block::Block) -> ResourcePrice {
    block.header().l1_gas_price.into()
}

pub(crate) fn starknet_version(block: &mp_block::Block) -> String {
    block.header().protocol_version.from_utf8().expect("starknet version should be a valid utf8 string")
}
