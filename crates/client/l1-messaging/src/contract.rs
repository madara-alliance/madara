use dc_eth::client::StarknetCoreContract::LogMessageToL2;
use dc_eth::utils::u256_to_felt;
use starknet_api::core::{ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

pub fn parse_handle_l1_message_transaction(event: &LogMessageToL2) -> anyhow::Result<L1HandlerTransaction> {
    // L1 from address.
    let from_address = u256_to_felt(event.fromAddress.into_word().into())?;

    // L2 contract to call.
    let contract_address = u256_to_felt(event.toAddress)?;

    // Function of the contract to call.
    let entry_point_selector = u256_to_felt(event.selector)?;

    // L1 message nonce.
    let nonce = u256_to_felt(event.nonce)?;

    let event_payload =
        event.payload.clone().into_iter().map(|param| u256_to_felt(param)).collect::<anyhow::Result<Vec<_>>>()?;

    let calldata: Calldata = {
        let mut calldata: Vec<_> = Vec::with_capacity(event.payload.len() + 1);
        calldata.push(from_address);
        calldata.extend(event_payload);

        Calldata(Arc::new(calldata))
    };

    Ok(L1HandlerTransaction {
        nonce: Nonce(nonce),
        contract_address: ContractAddress(contract_address.try_into()?),
        entry_point_selector: EntryPointSelector(entry_point_selector),
        calldata,
        version: TransactionVersion(Felt::ZERO),
    })
}
