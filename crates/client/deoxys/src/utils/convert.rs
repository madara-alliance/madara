//! Converts types from [`starknet_providers`] to madara's expected types.

use std::sync::Arc;

use mp_fee::ResourcePrice;
use mp_felt::Felt252Wrapper;
use sp_runtime::traits::Block as BlockT;
use starknet_api::hash::StarkFelt;
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models as p;

use crate::commitments::lib::calculate_commitments;

pub async fn block<B: BlockT>(block: p::Block, backend: Arc<mc_db::Backend<B>>) -> mp_block::Block {
    let count_tx = block.transactions.len() as u128;

    let mp_txs = conv_txs(block.transactions);
    let mp_block_number = block.block_number.expect("no block number provided");
    let mp_events = events(&block.transaction_receipts);
    let (transaction_commitment, event_commitment) = commitments(&mp_txs, &mp_events, mp_block_number, backend).await;

    let header = mp_block::Header {
        parent_block_hash: felt(block.parent_block_hash),
        block_number: mp_block_number,
        block_timestamp: block.timestamp,
        global_state_root: felt(block.state_root.expect("no state root provided")),
        sequencer_address: block.sequencer_address.map_or(contract_address(FieldElement::ZERO), contract_address),
        transaction_count: count_tx,
        transaction_commitment,
        event_count: mp_events.len() as u128,
        event_commitment,
        protocol_version: starknet_version(&block.starknet_version),
        l1_gas_price: resource_price(block.eth_l1_gas_price),
        extra_data: block.block_hash.map(|h| sp_core::U256::from_big_endian(&h.to_bytes_be())),
    };

    let ordered_events: Vec<mp_block::OrderedEvents> = block
        .transaction_receipts
        .iter()
        .enumerate()
        .filter(|(_, r)| r.events.len() > 0)
        .map(|(i, r)| mp_block::OrderedEvents::new(i as u128, r.events.iter().map(event).collect()))
        .collect();

    mp_block::Block::new(header, mp_txs, ordered_events)
}

fn conv_txs(txs: Vec<p::TransactionType>) -> Vec<mp_transactions::Transaction> {
    txs.into_iter().map(conv_tx).collect()
}

fn conv_tx(transaction: p::TransactionType) -> mp_transactions::Transaction {
    match transaction {
        p::TransactionType::InvokeFunction(tx) => mp_transactions::Transaction::Invoke(conv_tx_invoke(tx)),
        p::TransactionType::Declare(tx) => mp_transactions::Transaction::Declare(conv_tx_declare(tx)),
        p::TransactionType::Deploy(tx) => mp_transactions::Transaction::Deploy(conv_tx_deploy(tx)),
        p::TransactionType::DeployAccount(tx) => {
            mp_transactions::Transaction::DeployAccount(conv_tx_deploy_account(tx))
        }
        p::TransactionType::L1Handler(tx) => mp_transactions::Transaction::L1Handler(conv_tx_l1_handler(tx)),
    }
}

fn conv_tx_invoke(tx: p::InvokeFunctionTransaction) -> mp_transactions::InvokeTransaction {
    if tx.version == FieldElement::ZERO {
        mp_transactions::InvokeTransaction::V0(mp_transactions::InvokeTransactionV0 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
            contract_address: felt(tx.sender_address).into(),
            entry_point_selector: felt(tx.entry_point_selector.expect("no entry_point_selector provided")).into(),
            calldata: tx.calldata.into_iter().map(felt).map(Into::into).collect(),
        })
    } else {
        mp_transactions::InvokeTransaction::V1(mp_transactions::InvokeTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
            nonce: felt(tx.nonce.expect("no nonce provided")).into(),
            sender_address: felt(tx.sender_address).into(),
            calldata: tx.calldata.into_iter().map(felt).map(Into::into).collect(),
            offset_version: false,
        })
    }
}

fn conv_tx_declare(tx: p::DeclareTransaction) -> mp_transactions::DeclareTransaction {
    if tx.version == FieldElement::ZERO {
        mp_transactions::DeclareTransaction::V0(mp_transactions::DeclareTransactionV0 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
            nonce: felt(tx.nonce).into(),
            class_hash: felt(tx.class_hash).into(),
            sender_address: felt(tx.sender_address).into(),
        })
    } else if tx.version == FieldElement::ONE {
        mp_transactions::DeclareTransaction::V1(mp_transactions::DeclareTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
            nonce: felt(tx.nonce).into(),
            class_hash: felt(tx.class_hash).into(),
            sender_address: felt(tx.sender_address).into(),
            offset_version: false,
        })
    } else {
        mp_transactions::DeclareTransaction::V2(mp_transactions::DeclareTransactionV2 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
            nonce: felt(tx.nonce).into(),
            class_hash: felt(tx.class_hash).into(),
            sender_address: felt(tx.sender_address).into(),
            compiled_class_hash: felt(tx.compiled_class_hash.expect("no class hash available")).into(),
            offset_version: false,
        })
    }
}

fn conv_tx_deploy(tx: p::DeployTransaction) -> mp_transactions::DeployTransaction {
    mp_transactions::DeployTransaction {
        version: starknet_api::transaction::TransactionVersion(felt(tx.version)),
        class_hash: felt(tx.class_hash).into(),
        contract_address: felt(tx.contract_address).into(),
        contract_address_salt: felt(tx.contract_address_salt).into(),
        constructor_calldata: tx.constructor_calldata.into_iter().map(felt).map(Into::into).collect(),
    }
}

fn conv_tx_deploy_account(tx: p::DeployAccountTransaction) -> mp_transactions::DeployAccountTransaction {
    mp_transactions::DeployAccountTransaction {
        max_fee: fee(tx.max_fee.expect("no max fee provided")),
        signature: tx.signature.into_iter().map(felt).map(Into::into).collect(),
        nonce: felt(tx.nonce).into(),
        contract_address_salt: felt(tx.contract_address_salt).into(),
        constructor_calldata: tx.constructor_calldata.into_iter().map(felt).map(Into::into).collect(),
        class_hash: felt(tx.class_hash).into(),
        offset_version: false,
    }
}

fn conv_tx_l1_handler(tx: p::L1HandlerTransaction) -> mp_transactions::HandleL1MessageTransaction {
    mp_transactions::HandleL1MessageTransaction {
        nonce: tx
            .nonce
            .ok_or("Nonce value is missing")
            .and_then(|n| u64::try_from(felt(n)).map_err(|_| "Failed to convert felt value to u64"))
            .unwrap_or_else(|e| {
                eprintln!("{}", e);
                0
            }),
        contract_address: felt(tx.contract_address).into(),
        entry_point_selector: felt(tx.entry_point_selector).into(),
        calldata: tx.calldata.into_iter().map(felt).map(Into::into).collect(),
    }
}

/// Converts a starknet version string to a felt value.
/// If the string contains more than 31 bytes, the function panics.
fn starknet_version(version: &Option<String>) -> Felt252Wrapper {
    match version {
        Some(version) => {
            Felt252Wrapper::try_from(version.as_bytes()).expect("Failed to convert version to felt: string is too long")
        }
        None => Felt252Wrapper::ZERO,
    }
}

fn fee(felt: starknet_ff::FieldElement) -> u128 {
    felt.try_into().expect("Value out of range for u128")
}

fn resource_price(eth_l1_gas_price: starknet_ff::FieldElement) -> ResourcePrice {
    ResourcePrice {
        price_in_strk: None,
        price_in_wei: fee(eth_l1_gas_price).try_into().expect("Value out of range for u64"),
    }
}

fn events(receipts: &[p::ConfirmedTransactionReceipt]) -> Vec<starknet_api::transaction::Event> {
    receipts.iter().flat_map(|r| &r.events).map(event).collect()
}

fn event(event: &p::Event) -> starknet_api::transaction::Event {
    use starknet_api::transaction::{Event, EventContent, EventData, EventKey};

    Event {
        from_address: contract_address(event.from_address),
        content: EventContent {
            keys: event.keys.iter().copied().map(felt).map(EventKey).collect(),
            data: EventData(event.data.iter().copied().map(felt).collect()),
        },
    }
}

async fn commitments<B: BlockT>(
    transactions: &[mp_transactions::Transaction],
    events: &[starknet_api::transaction::Event],
    block_number: u64,
    backend: Arc<mc_db::Backend<B>>,
) -> (StarkFelt, StarkFelt) {
    use mp_hashers::pedersen::PedersenHasher;

    let chain_id = chain_id();

    let (commitment_tx, commitment_event) =
        calculate_commitments::<B, PedersenHasher>(transactions, events, chain_id, block_number, backend).await;

    (commitment_tx.into(), commitment_event.into())
}

fn chain_id() -> mp_felt::Felt252Wrapper {
    starknet_ff::FieldElement::from_byte_slice_be(b"SN_MAIN").unwrap().into()
}

fn felt(field_element: starknet_ff::FieldElement) -> starknet_api::hash::StarkFelt {
    starknet_api::hash::StarkFelt::new(field_element.to_bytes_be()).unwrap()
}

fn contract_address(field_element: starknet_ff::FieldElement) -> starknet_api::api_core::ContractAddress {
    starknet_api::api_core::ContractAddress(starknet_api::api_core::PatriciaKey(felt(field_element)))
}
