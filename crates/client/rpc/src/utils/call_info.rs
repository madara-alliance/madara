use blockifier::execution::call_info::CallInfo;
use dp_convert::core_felt::CoreFelt;
use starknet_core::types::{
    ComputationResources, DataAvailabilityResources, DataResources, Event, ExecutionResources, Felt, MsgToL1,
};

pub(crate) fn extract_events_from_call_info(call_info: &CallInfo) -> Vec<Event> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .events
        .iter()
        .map(|ordered_event| Event {
            from_address: address.into_core_felt(),
            keys: ordered_event.event.keys.iter().map(|key| key.0.into_core_felt()).collect(),
            data: ordered_event.event.data.0.iter().map(CoreFelt::into_core_felt).collect(),
        })
        .collect();

    let inner_events: Vec<_> = call_info.inner_calls.iter().flat_map(extract_events_from_call_info).collect();

    events.into_iter().chain(inner_events).collect()
}

pub(crate) fn extract_messages_from_call_info(call_info: &CallInfo) -> Vec<MsgToL1> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .map(|msg| MsgToL1 {
            from_address: address.into_core_felt(),
            to_address: Felt::from_bytes_be_slice(msg.message.to_address.0.to_fixed_bytes().as_slice()),
            payload: msg.message.payload.0.iter().map(CoreFelt::into_core_felt).collect(),
        })
        .collect();

    let inner_messages: Vec<_> = call_info.inner_calls.iter().flat_map(extract_messages_from_call_info).collect();

    events.into_iter().chain(inner_messages).collect()
}

pub(crate) fn blockifier_call_info_to_starknet_resources(callinfo: &CallInfo) -> ExecutionResources {
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
