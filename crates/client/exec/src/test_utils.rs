use std::{collections::HashMap, sync::Arc};

use dc_db::DeoxysBackend;
use dp_block::{
    chain_config::ChainConfig, DeoxysBlockInfo, DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo, Header,
};
use dp_class::{ClassInfo, ContractClass, ConvertedClass, ToCompiledClass};
use dp_state_update::{ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, StateDiff, StorageEntry};
use rstest::rstest;
use starknet_core::types::contract::{legacy::LegacyContractClass, SierraClass};
use starknet_types_core::felt::Felt;

const UDC_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

pub struct InitialBalance {
    pub eth_wei: u128,
    pub strk_fri: u128,
}

struct GenesisBuilder {
    pub initial_balances: HashMap<Felt, InitialBalance>,
}

lazy_static::lazy_static! {
    pub static ref UDC_CLASS: ConvertedClass = load_legacy_class(
        include_bytes!("../resources/udc.json"),
    );
    pub static ref ERC20_CLASS: ConvertedClass = load_legacy_class(
        include_bytes!("../resources/erc20.json"),
    );
    pub static ref DUMMY_CLASS: ConvertedClass = load_sierra_class(
        include_bytes!("../resources/dummy_account.json"),
    );
}

pub fn load_sierra_class(class: &[u8]) -> ConvertedClass {
    let class = serde_json::from_slice::<SierraClass>(class).unwrap().flatten().unwrap();
    let class_hash = class.class_hash();
    let compiled_class_hash = class.class_hash();
    let class = ContractClass::Sierra(class.into());
    let compiled = class.compile().unwrap();
    ConvertedClass {
        class_infos: (class_hash, ClassInfo { contract_class: class, compiled_class_hash, block_number: Some(0) }),
        class_compiled: (compiled_class_hash, compiled),
    }
}
pub fn load_legacy_class(class: &[u8]) -> ConvertedClass {
    let class = serde_json::from_slice::<LegacyContractClass>(class).unwrap();
    let class_hash = class.class_hash().unwrap();
    let compiled_class_hash = class.class_hash().unwrap();
    let class = ContractClass::Legacy(class.compress().unwrap().into());
    let compiled = class.compile().unwrap();
    ConvertedClass {
        class_infos: (class_hash, ClassInfo { contract_class: class, compiled_class_hash, block_number: Some(0) }),
        class_compiled: (compiled_class_hash, compiled),
    }
}

pub fn make_genesis(backend: &DeoxysBackend) {
    let dummy_contract_addr = Felt::from_hex_unchecked("0x99999999999999999");
    let eth_contract = **backend.chain_config().parent_fee_token_address;
    let strk_contract = **backend.chain_config().native_fee_token_address;

    let initial_balances: HashMap<Felt, InitialBalance> =
        [(dummy_contract_addr, InitialBalance { eth_wei: 1_000, strk_fri: 128_000 })].into();

    let mut storage_diffs = HashMap::new();
    for (contract, bal) in initial_balances {
        let low_key = blockifier::abi::abi_utils::get_fee_token_var_address(contract.try_into().unwrap());
        let high_key = blockifier::abi::sierra_types::next_storage_key(&low_key).unwrap();

        let item = storage_diffs.entry(eth_contract).or_insert(HashMap::new());
        item.insert(**low_key, bal.eth_wei.into());
        item.insert(**high_key, Felt::ZERO);

        let item = storage_diffs.entry(strk_contract).or_insert(HashMap::new());
        item.insert(**low_key, bal.strk_fri.into());
        item.insert(**high_key, Felt::ZERO);
    }

    let storage_diffs = storage_diffs
        .into_iter()
        .map(|(address, storage_entries)| ContractStorageDiffItem {
            address,
            storage_entries: storage_entries.into_iter().map(|(key, value)| StorageEntry { key, value }).collect(),
        })
        .collect();

    let classes = vec![ERC20_CLASS.clone(), UDC_CLASS.clone(), DUMMY_CLASS.clone()];

    let deployed_contracts = vec![
        DeployedContractItem { address: eth_contract, class_hash: ERC20_CLASS.class_infos.0 },
        DeployedContractItem { address: strk_contract, class_hash: ERC20_CLASS.class_infos.0 },
        DeployedContractItem { address: UDC_ADDRESS, class_hash: UDC_CLASS.class_infos.0 },
        DeployedContractItem { address: dummy_contract_addr, class_hash: DUMMY_CLASS.class_infos.0 },
    ];

    backend
        .store_block(
            DeoxysMaybePendingBlock {
                info: DeoxysMaybePendingBlockInfo::NotPending(DeoxysBlockInfo {
                    header: Header { parent_block_hash: Felt::ZERO, block_number: 0, ..Default::default() },
                    block_hash: Felt::ZERO,
                    tx_hashes: Default::default(),
                }),
                inner: Default::default(),
            },
            StateDiff {
                storage_diffs,
                deprecated_declared_classes: Default::default(),
                declared_classes: classes
                    .iter()
                    .map(|c| DeclaredClassItem { class_hash: c.class_infos.0, compiled_class_hash: c.class_compiled.0 })
                    .collect(),
                deployed_contracts,
                replaced_classes: Default::default(),
                nonces: Default::default(),
            },
            classes,
        )
        .unwrap();
}

#[rstest]
fn test_make_genesis() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = DeoxysBackend::open_for_testing(chain_config.clone());

    make_genesis(&backend);
}
