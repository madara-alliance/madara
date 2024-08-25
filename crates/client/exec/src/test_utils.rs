use std::{collections::HashMap, sync::Arc};
use dc_db::DeoxysBackend;
use dp_block::Header;
use dp_state_update::StateDiff;
use starknet_types_core::felt::Felt;
use dp_chain_config::ChainConfig;

fn initial_balances_to_storage_diffs(
    chain_config: &ChainConfig,
    balances: HashMap<Felt, InitialBalance>,
    storage_diffs: &mut HashMap<Felt, HashMap<Felt, Felt>>,
) {
    for (contract_address, bal) in balances {
        // Storage key where the balance of that contract is stored. For both STRK and ETH it ends up
        // being the same key.
        let low_key = blockifier::abi::abi_utils::get_fee_token_var_address(contract_address.try_into().unwrap());
        // let high_key = blockifier::abi::sierra_types::next_storage_key(&low_key)?;

        // ETH erc20
        let erc20_contract = chain_config.parent_fee_token_address;
        let kv = storage_diffs.entry(**erc20_contract).or_insert(HashMap::new());
        kv.insert(**low_key, bal.wei.into());
        // kv.insert(high_key, Felt::ZERO);

        // STRK erc20
        let erc20_contract = chain_config.native_fee_token_address;
        let kv = storage_diffs.entry(**erc20_contract).or_insert(HashMap::new());
        kv.insert(**low_key, bal.fri.into());
        // kv.insert(high_key, Felt::ZERO);
    }
}

pub struct InitialBalance {
    pub fri: Felt,
    pub wei: Felt,
}

pub fn make_genesis_block() {
    let chain_config = Arc::new(ChainConfig::test_config());
    let backend = DeoxysBackend::open_for_testing(chain_config);

    let contract_1 = Felt::TWO;

    let initial_balances: HashMap<_, _> =
        [(contract_1, InitialBalance { fri: 10_000.into(), wei: 10_000.into() })].into();

    backend
        .store_block(
            dp_block::DeoxysMaybePendingBlock {
                info: dp_block::DeoxysMaybePendingBlockInfo::NotPending(dp_block::DeoxysBlockInfo {
                    header: Header {
                        parent_block_hash: Felt::ZERO,
                        block_number: 0,
                        l1_gas_price: dp_block::header::GasPrices {
                            eth_l1_gas_price: 10,
                            strk_l1_gas_price: 8,
                            eth_l1_data_gas_price: 9,
                            strk_l1_data_gas_price: 7,
                        },
                        l1_da_mode: dp_block::header::L1DataAvailabilityMode::Blob,
                        ..Default::default()
                    },
                    block_hash: Felt::ONE,
                    tx_hashes: vec![],
                }),
                inner: dp_block::DeoxysBlockInner::default(),
            },
            StateDiff {
                storage_diffs: todo!(),
                deprecated_declared_classes: todo!(),
                declared_classes: todo!(),
                deployed_contracts: todo!(),
                replaced_classes: todo!(),
                nonces: todo!(),
            },
            converted_classes,
        )
        .unwrap()
}
