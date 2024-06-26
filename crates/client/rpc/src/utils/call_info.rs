use blockifier::execution::call_info::CallInfo;
use dp_convert::ToFelt;
use starknet_core::types::{Event, Felt, MsgToL1};

#[allow(unused)]
pub(crate) fn extract_events_from_call_info(call_info: &CallInfo) -> Vec<Event> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .events
        .iter()
        .map(|ordered_event| Event {
            from_address: address.to_felt(),
            keys: ordered_event.event.keys.iter().map(|key| key.0.to_felt()).collect(),
            data: ordered_event.event.data.0.iter().map(ToFelt::to_felt).collect(),
        })
        .collect();

    let inner_events: Vec<_> = call_info.inner_calls.iter().flat_map(extract_events_from_call_info).collect();

    events.into_iter().chain(inner_events).collect()
}

#[allow(unused)]
pub(crate) fn extract_messages_from_call_info(call_info: &CallInfo) -> Vec<MsgToL1> {
    let address = call_info.call.storage_address;
    let events: Vec<_> = call_info
        .execution
        .l2_to_l1_messages
        .iter()
        .map(|msg| MsgToL1 {
            from_address: address.to_felt(),
            to_address: Felt::from_bytes_be_slice(msg.message.to_address.0.to_fixed_bytes().as_slice()),
            payload: msg.message.payload.0.iter().map(ToFelt::to_felt).collect(),
        })
        .collect();

    let inner_messages: Vec<_> = call_info.inner_calls.iter().flat_map(extract_messages_from_call_info).collect();

    events.into_iter().chain(inner_messages).collect()
}
