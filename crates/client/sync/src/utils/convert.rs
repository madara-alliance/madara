//! Converts types from [`starknet_providers`] to deoxys's expected types.

use std::collections::HashMap;
use std::num::NonZeroU128;
use std::str::FromStr;
use std::sync::Arc;

use blockifier::block::GasPrices;
use dp_block::{DeoxysBlock, DeoxysBlockInfo, DeoxysBlockInner, StarknetVersion};
use dp_convert::to_stark_felt::ToStarkFelt;
use dp_transactions::from_broadcasted_transactions::fee_from_felt;
use starknet_api::block::BlockHash;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{
    DeclareTransaction, DeployAccountTransaction, DeployAccountTransactionV1, DeployTransaction, Event,
    InvokeTransaction, L1HandlerTransaction, Transaction, TransactionHash,
};
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff as StateDiffCore, StorageEntry,
};
use starknet_providers::sequencer::models::state_update::{
    DeclaredContract, DeployedContract, StateDiff as StateDiffProvider, StorageDiff as StorageDiffProvider,
};
use starknet_providers::sequencer::models::{self as p, StateUpdate as StateUpdateProvider};
use starknet_types_core::felt::Felt;

use crate::commitments::calculate_tx_and_event_commitments;
use crate::l2::L2SyncError;

pub struct ConvertedBlock {
    pub block: DeoxysBlock,
    pub reverted_txs: Vec<TransactionHash>,
}

/// Compute heavy, this should only be called in a rayon ctx
pub fn convert_block(block: p::Block, chain_id: Felt) -> Result<ConvertedBlock, L2SyncError> {
    // converts starknet_provider transactions and events to dp_transactions and starknet_api events
    let transactions = transactions(block.transactions);
    let reverted_transactions = reverted_transactions(&block.transaction_receipts);
    let events = events(&block.transaction_receipts);
    let parent_block_hash = block.parent_block_hash.to_stark_felt();
    let block_hash = block.block_hash.expect("no block hash provided");
    let block_number = block.block_number.expect("no block number provided");
    let block_timestamp = block.timestamp;
    let global_state_root = block.state_root.expect("no state root provided").to_stark_felt();
    let sequencer_address = block.sequencer_address.map_or(contract_address(Felt::ZERO), contract_address);
    let transaction_count = transactions.len() as u128;
    let event_count = events.len() as u128;

    let ((transaction_commitment, txs_hashes), event_commitment) =
        calculate_tx_and_event_commitments(&transactions, &events, chain_id, block_number);

    // Provisory conversion while Starknet-api doesn't support the universal `Felt` type
    let transaction_commitment = transaction_commitment.to_stark_felt();
    let event_commitment = event_commitment.to_stark_felt();
    let txs_hashes: Vec<StarkFelt> = txs_hashes.iter().map(|felt| (*felt).to_stark_felt()).collect();

    let protocol_version = protocol_version(block.starknet_version);
    let l1_gas_price = resource_price(block.l1_gas_price, block.l1_data_gas_price);
    let l1_da_mode = l1_da_mode(block.l1_da_mode);
    let extra_data = Some(dp_block::U256::from_big_endian(&block_hash.to_bytes_be()));

    let header = dp_block::Header {
        parent_block_hash,
        block_number,
        block_timestamp,
        global_state_root,
        sequencer_address,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
        extra_data,
    };

    let computed_block_hash = header.hash(chain_id);
    // mismatched block hash is allowed for blocks 1466..=2242
    if computed_block_hash != block_hash && !(1466..=2242).contains(&block_number) {
        return Err(L2SyncError::MismatchedBlockHash(block_number));
    }
    let ordered_events: Vec<dp_block::OrderedEvents> = block
        .transaction_receipts
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.events.is_empty())
        .map(|(i, r)| dp_block::OrderedEvents::new(i as u128, r.events.iter().map(event).collect()))
        .collect();

    let block = DeoxysBlock::new(
        DeoxysBlockInfo::new(
            header,
            txs_hashes.into_iter().map(TransactionHash).collect(),
            BlockHash(block_hash.to_stark_felt()),
        ),
        DeoxysBlockInner::new(transactions, ordered_events),
    );

    Ok(ConvertedBlock { block, reverted_txs: reverted_transactions })
}

fn protocol_version(version: Option<String>) -> StarknetVersion {
    version.map(|version| StarknetVersion::from_str(&version).unwrap_or_default()).unwrap_or_default()
}

fn transactions(txs: Vec<p::TransactionType>) -> Vec<Transaction> {
    txs.into_iter().map(transaction).collect()
}

fn transaction(transaction: p::TransactionType) -> Transaction {
    match transaction {
        p::TransactionType::Declare(tx) => Transaction::Declare(declare_transaction(tx)),
        p::TransactionType::Deploy(tx) => Transaction::Deploy(deploy_transaction(tx)),
        p::TransactionType::DeployAccount(tx) => Transaction::DeployAccount(deploy_account_transaction(tx)),
        p::TransactionType::InvokeFunction(tx) => Transaction::Invoke(invoke_transaction(tx)),
        p::TransactionType::L1Handler(tx) => Transaction::L1Handler(l1_handler_transaction(tx)),
    }
}

fn declare_transaction(tx: p::DeclareTransaction) -> DeclareTransaction {
    if tx.version == Felt::ZERO {
        DeclareTransaction::V0(starknet_api::transaction::DeclareTransactionV0V1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            sender_address: contract_address(tx.sender_address),
        })
    } else if tx.version == Felt::ONE {
        DeclareTransaction::V1(starknet_api::transaction::DeclareTransactionV0V1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            sender_address: contract_address(tx.sender_address),
        })
    } else if tx.version == Felt::TWO {
        DeclareTransaction::V2(starknet_api::transaction::DeclareTransactionV2 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            compiled_class_hash: compiled_class_hash(tx.compiled_class_hash.expect("no compiled class hash provided")),
            sender_address: contract_address(tx.sender_address),
        })
    } else if tx.version == Felt::THREE {
        DeclareTransaction::V3(starknet_api::transaction::DeclareTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            compiled_class_hash: compiled_class_hash(tx.compiled_class_hash.expect("no compiled class hash provided")),
            sender_address: contract_address(tx.sender_address),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
            account_deployment_data: account_deployment_data(
                tx.account_deployment_data.expect("no account_deployment_data provided"),
            ),
        })
    } else {
        panic!("declare transaction version not supported");
    }
}

fn deploy_transaction(tx: p::DeployTransaction) -> DeployTransaction {
    DeployTransaction {
        version: transaction_version(tx.version),
        class_hash: class_hash(tx.class_hash),
        contract_address_salt: contract_address_salt(tx.contract_address_salt),
        constructor_calldata: call_data(tx.constructor_calldata),
    }
}

fn deploy_account_transaction(tx: p::DeployAccountTransaction) -> DeployAccountTransaction {
    match deploy_account_transaction_version(&tx) {
        1 => DeployAccountTransaction::V1(DeployAccountTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            contract_address_salt: contract_address_salt(tx.contract_address_salt),
            constructor_calldata: call_data(tx.constructor_calldata),
        }),

        3 => DeployAccountTransaction::V3(starknet_api::transaction::DeployAccountTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            contract_address_salt: contract_address_salt(tx.contract_address_salt),
            constructor_calldata: call_data(tx.constructor_calldata),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
        }),

        _ => panic!("deploy account transaction version not supported"),
    }
}

// TODO: implement something better than this
fn deploy_account_transaction_version(tx: &p::DeployAccountTransaction) -> u8 {
    if tx.resource_bounds.is_some() {
        3
    } else {
        1
    }
}

fn invoke_transaction(tx: p::InvokeFunctionTransaction) -> InvokeTransaction {
    if tx.version == Felt::ZERO {
        InvokeTransaction::V0(starknet_api::transaction::InvokeTransactionV0 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            contract_address: contract_address(tx.sender_address),
            entry_point_selector: entry_point(tx.entry_point_selector.expect("no entry_point_selector provided")),
            calldata: call_data(tx.calldata),
        })
    } else if tx.version == Felt::ONE {
        InvokeTransaction::V1(starknet_api::transaction::InvokeTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce.expect("no nonce provided")),
            sender_address: contract_address(tx.sender_address),
            calldata: call_data(tx.calldata),
        })
    } else if tx.version == Felt::THREE {
        InvokeTransaction::V3(starknet_api::transaction::InvokeTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce.expect("no nonce provided")),
            sender_address: contract_address(tx.sender_address),
            calldata: call_data(tx.calldata),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
            account_deployment_data: account_deployment_data(
                tx.account_deployment_data.expect("no account_deployment_data provided"),
            ),
        })
    } else {
        panic!("invoke transaction version not supported");
    }
}

fn l1_handler_transaction(tx: p::L1HandlerTransaction) -> L1HandlerTransaction {
    L1HandlerTransaction {
        version: transaction_version(tx.version),
        nonce: nonce(tx.nonce.unwrap_or_default()), // TODO check when a L1Ha
        contract_address: contract_address(tx.contract_address),
        entry_point_selector: entry_point(tx.entry_point_selector),
        calldata: call_data(tx.calldata),
    }
}

fn reverted_transactions(receipts: &[p::ConfirmedTransactionReceipt]) -> Vec<TransactionHash> {
    receipts
        .iter()
        .filter(|r| r.execution_status == Some(p::TransactionExecutionStatus::Reverted))
        .map(|r| TransactionHash(r.transaction_hash.to_stark_felt()))
        .collect()
}

fn fee(fee: Felt) -> starknet_api::transaction::Fee {
    fee_from_felt(fee)
}

fn signature(signature: Vec<Felt>) -> starknet_api::transaction::TransactionSignature {
    starknet_api::transaction::TransactionSignature(signature.into_iter().map(ToStarkFelt::to_stark_felt).collect())
}

fn contract_address(address: Felt) -> starknet_api::core::ContractAddress {
    address.to_stark_felt().try_into().unwrap()
}

fn entry_point(entry_point: Felt) -> starknet_api::core::EntryPointSelector {
    starknet_api::core::EntryPointSelector(entry_point.to_stark_felt())
}

fn call_data(call_data: Vec<Felt>) -> starknet_api::transaction::Calldata {
    starknet_api::transaction::Calldata(Arc::new(call_data.into_iter().map(ToStarkFelt::to_stark_felt).collect()))
}

fn nonce(nonce: Felt) -> starknet_api::core::Nonce {
    starknet_api::core::Nonce(nonce.to_stark_felt())
}

fn class_hash(class_hash: Felt) -> starknet_api::core::ClassHash {
    starknet_api::core::ClassHash(class_hash.to_stark_felt())
}

fn compiled_class_hash(compiled_class_hash: Felt) -> starknet_api::core::CompiledClassHash {
    starknet_api::core::CompiledClassHash(compiled_class_hash.to_stark_felt())
}

fn contract_address_salt(contract_address_salt: Felt) -> starknet_api::transaction::ContractAddressSalt {
    starknet_api::transaction::ContractAddressSalt(contract_address_salt.to_stark_felt())
}

fn transaction_version(version: Felt) -> starknet_api::transaction::TransactionVersion {
    starknet_api::transaction::TransactionVersion(version.to_stark_felt())
}

fn resource_bounds(
    ressource_bounds: starknet_providers::sequencer::models::ResourceBoundsMapping,
) -> starknet_api::transaction::ResourceBoundsMapping {
    starknet_api::transaction::ResourceBoundsMapping::try_from(vec![
        (
            starknet_api::transaction::Resource::L1Gas,
            starknet_api::transaction::ResourceBounds {
                max_amount: ressource_bounds.l1_gas.max_amount,
                max_price_per_unit: ressource_bounds.l1_gas.max_price_per_unit,
            },
        ),
        (
            starknet_api::transaction::Resource::L2Gas,
            starknet_api::transaction::ResourceBounds {
                max_amount: ressource_bounds.l2_gas.max_amount,
                max_price_per_unit: ressource_bounds.l2_gas.max_price_per_unit,
            },
        ),
    ])
    .expect("Failed to convert resource bounds")
}

fn tip(tip: u64) -> starknet_api::transaction::Tip {
    starknet_api::transaction::Tip(tip)
}

fn data_availability_mode(
    mode: starknet_providers::sequencer::models::DataAvailabilityMode,
) -> starknet_api::data_availability::DataAvailabilityMode {
    match mode {
        starknet_providers::sequencer::models::DataAvailabilityMode::L1 => {
            starknet_api::data_availability::DataAvailabilityMode::L1
        }
        starknet_providers::sequencer::models::DataAvailabilityMode::L2 => {
            starknet_api::data_availability::DataAvailabilityMode::L2
        }
    }
}

fn paymaster_data(paymaster_data: Vec<Felt>) -> starknet_api::transaction::PaymasterData {
    starknet_api::transaction::PaymasterData(paymaster_data.into_iter().map(ToStarkFelt::to_stark_felt).collect())
}

fn account_deployment_data(account_deployment_data: Vec<Felt>) -> starknet_api::transaction::AccountDeploymentData {
    starknet_api::transaction::AccountDeploymentData(
        account_deployment_data.into_iter().map(ToStarkFelt::to_stark_felt).collect(),
    )
}

/// Converts the l1 gas price and l1 data gas price to a GasPrices struct, if the l1 gas price is
/// not 0. If the l1 gas price is 0, returns None.
/// The other prices are converted to NonZeroU128, with 0 being converted to 1.
fn resource_price(
    l1_gas_price: starknet_core::types::ResourcePrice,
    l1_data_gas_price: starknet_core::types::ResourcePrice,
) -> Option<GasPrices> {
    /// Converts a Felt to a NonZeroU128, with 0 being converted to 1.
    fn felt_to_non_zero_u128(felt: Felt) -> NonZeroU128 {
        let value: u128 = if felt == Felt::ZERO { 1 } else { fee_from_felt(felt).0 };
        NonZeroU128::new(value).expect("Failed to convert field_element to NonZeroU128")
    }

    if l1_gas_price.price_in_wei == Felt::ZERO {
        None
    } else {
        Some(GasPrices {
            eth_l1_gas_price: felt_to_non_zero_u128(l1_gas_price.price_in_wei),
            strk_l1_gas_price: felt_to_non_zero_u128(l1_gas_price.price_in_fri),
            eth_l1_data_gas_price: felt_to_non_zero_u128(l1_data_gas_price.price_in_wei),
            strk_l1_data_gas_price: felt_to_non_zero_u128(l1_data_gas_price.price_in_fri),
        })
    }
}

fn l1_da_mode(
    mode: starknet_core::types::L1DataAvailabilityMode,
) -> starknet_api::data_availability::L1DataAvailabilityMode {
    match mode {
        starknet_core::types::L1DataAvailabilityMode::Calldata => {
            starknet_api::data_availability::L1DataAvailabilityMode::Calldata
        }
        starknet_core::types::L1DataAvailabilityMode::Blob => {
            starknet_api::data_availability::L1DataAvailabilityMode::Blob
        }
    }
}

fn events(receipts: &[p::ConfirmedTransactionReceipt]) -> Vec<starknet_api::transaction::Event> {
    receipts.iter().flat_map(|r| &r.events).map(event).collect()
}

fn event(event: &p::Event) -> starknet_api::transaction::Event {
    use starknet_api::transaction::{EventContent, EventData, EventKey};

    Event {
        from_address: contract_address(event.from_address),
        content: EventContent {
            keys: event.keys.iter().copied().map(ToStarkFelt::to_stark_felt).map(EventKey).collect(),
            data: EventData(event.data.iter().copied().map(ToStarkFelt::to_stark_felt).collect()),
        },
    }
}

pub fn state_update(state_update: StateUpdateProvider) -> PendingStateUpdate {
    let old_root = state_update.old_root;
    let state_diff = state_diff(state_update.state_diff);

    // StateUpdateCore { block_hash, old_root, new_root, state_diff }
    PendingStateUpdate { old_root, state_diff }
}

fn state_diff(state_diff: StateDiffProvider) -> StateDiffCore {
    let storage_diffs = storage_diffs(state_diff.storage_diffs);
    let deprecated_declared_classes = state_diff.old_declared_contracts;
    let declared_classes = declared_classes(state_diff.declared_classes);
    let deployed_contracts = deployed_contracts(state_diff.deployed_contracts);
    let replaced_classes = replaced_classes(state_diff.replaced_classes);
    let nonces = nonces(state_diff.nonces);

    StateDiffCore {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts,
        replaced_classes,
        nonces,
    }
}

fn storage_diffs(storage_diffs: HashMap<Felt, Vec<StorageDiffProvider>>) -> Vec<ContractStorageDiffItem> {
    storage_diffs
        .into_iter()
        .map(|(address, entries)| ContractStorageDiffItem { address, storage_entries: storage_entries(entries) })
        .collect()
}

fn storage_entries(storage_entries: Vec<StorageDiffProvider>) -> Vec<StorageEntry> {
    storage_entries.into_iter().map(|StorageDiffProvider { key, value }| StorageEntry { key, value }).collect()
}

fn declared_classes(declared_classes: Vec<DeclaredContract>) -> Vec<DeclaredClassItem> {
    declared_classes
        .into_iter()
        .map(|DeclaredContract { class_hash, compiled_class_hash }| DeclaredClassItem {
            class_hash,
            compiled_class_hash,
        })
        .collect()
}

fn deployed_contracts(deplyed_contracts: Vec<DeployedContract>) -> Vec<DeployedContractItem> {
    deplyed_contracts
        .into_iter()
        .map(|DeployedContract { address, class_hash }| DeployedContractItem { address, class_hash })
        .collect()
}

fn replaced_classes(replaced_classes: Vec<DeployedContract>) -> Vec<ReplacedClassItem> {
    replaced_classes
        .into_iter()
        .map(|DeployedContract { address, class_hash }| ReplacedClassItem { contract_address: address, class_hash })
        .collect()
}

fn nonces(nonces: HashMap<Felt, Felt>) -> Vec<NonceUpdate> {
    // TODO: make sure the order is `contract_address` -> `nonce`
    // and not `nonce` -> `contract_address`
    nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect()
}
