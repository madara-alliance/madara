use std::num::NonZeroU128;

use dp_block::DeoxysBlock;
use starknet_core::types::{Felt, L1DataAvailabilityMode, ResourcePrice};

pub(crate) fn l1_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    // 1 is a special value that means 0 because the gas price is stored as a NonZeroU128
    fn non_zeo_u128_to_felt(value: NonZeroU128) -> Felt {
        match value.get() {
            1 => Felt::ZERO,
            x => Felt::from(x),
        }
    }

    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: non_zeo_u128_to_felt(resource_price.strk_l1_gas_price),
            price_in_wei: non_zeo_u128_to_felt(resource_price.eth_l1_gas_price),
        },
        None => ResourcePrice { price_in_fri: Felt::ZERO, price_in_wei: Felt::ZERO },
    }
}

pub(crate) fn l1_data_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: resource_price.strk_l1_data_gas_price.get().into(),
            price_in_wei: resource_price.eth_l1_data_gas_price.get().into(),
        },
        None => ResourcePrice { price_in_fri: Felt::ONE, price_in_wei: Felt::ONE },
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
    block.header().protocol_version.clone()
}
