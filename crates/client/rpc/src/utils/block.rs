use std::num::NonZeroU128;

use mp_block::DeoxysBlock;
use starknet_core::types::{FieldElement, L1DataAvailabilityMode, ResourcePrice};

pub(crate) fn parent_hash(block: &DeoxysBlock) -> FieldElement {
    // safe unwrap() FieldElement from Starkfelt
    FieldElement::from_bytes_be(&block.header().parent_block_hash.0).unwrap()
}

pub(crate) fn new_root(block: &DeoxysBlock) -> FieldElement {
    // safe unwrap() FieldElement from Starkfelt
    FieldElement::from_bytes_be(&block.header().global_state_root.0).unwrap()
}

pub(crate) fn timestamp(block: &DeoxysBlock) -> u64 {
    block.header().block_timestamp
}

pub(crate) fn sequencer_address(block: &DeoxysBlock) -> FieldElement {
    // safe unwrap() FieldElement from Starkfelt
    FieldElement::from_bytes_be(&block.header().sequencer_address.0 .0 .0).unwrap()
}

pub(crate) fn l1_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    // 1 is a special value that means 0 because the gas price is stored as a NonZeroU128
    fn non_zeo_u128_to_field_element(value: NonZeroU128) -> FieldElement {
        match value.get() {
            1 => FieldElement::ZERO,
            x => FieldElement::from(x),
        }
    }

    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: non_zeo_u128_to_field_element(resource_price.strk_l1_gas_price),
            price_in_wei: non_zeo_u128_to_field_element(resource_price.eth_l1_gas_price),
        },
        None => ResourcePrice { price_in_fri: FieldElement::ZERO, price_in_wei: FieldElement::ZERO },
    }
}

pub(crate) fn l1_data_gas_price(block: &DeoxysBlock) -> ResourcePrice {
    let resource_price = &block.header().l1_gas_price;

    match resource_price {
        Some(resource_price) => ResourcePrice {
            price_in_fri: resource_price.strk_l1_data_gas_price.get().into(),
            price_in_wei: resource_price.eth_l1_data_gas_price.get().into(),
        },
        None => ResourcePrice { price_in_fri: FieldElement::ONE, price_in_wei: FieldElement::ONE },
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
    block.header().protocol_version.from_utf8().expect("starknet version should be a valid utf8 string")
}
