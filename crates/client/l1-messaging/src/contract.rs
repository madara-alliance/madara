use dc_eth::client::StarknetCoreContract::LogMessageToL2;
use dc_eth::utils::{address_to_starkfelt, u256_to_starkfelt};
use starknet_api::core::{ContractAddress, EntryPointSelector, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use std::sync::Arc;

// Blockifier type :
//
// pub struct L1HandlerTransaction {
//     pub tx: starknet_api::transaction::L1HandlerTransaction,
//     pub tx_hash: TransactionHash,
//     pub paid_fee_on_l1: Fee,
// }

//starknet_api::transaction::L1HandlerTransaction:
//
// pub struct L1HandlerTransaction {
//     pub version: TransactionVersion,
//     pub nonce: Nonce,
//     pub contract_address: ContractAddress,
//     pub entry_point_selector: EntryPointSelector,
//     pub calldata: Calldata,
// }

pub fn parse_handle_l1_message_transaction(event: &LogMessageToL2) -> anyhow::Result<L1HandlerTransaction> {
    // L1 from address.
    let from_address = address_to_starkfelt(event.fromAddress)?;

    // L2 contract to call.
    let contract_address = u256_to_starkfelt(event.toAddress)?;

    // Function of the contract to call.
    let entry_point_selector = u256_to_starkfelt(event.selector)?;

    // L1 message nonce.
    let nonce = u256_to_starkfelt(event.nonce)?;

    let event_payload =
        event.payload.clone().into_iter().map(|param| u256_to_starkfelt(param)).collect::<anyhow::Result<Vec<_>>>()?;

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
        version: TransactionVersion(StarkFelt::ZERO),
    })
}
