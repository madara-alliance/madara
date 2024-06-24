use dp_block::{DeoxysBlock, L1DataAvailabilityMode};

pub(crate) fn l1_gas_price(block: &DeoxysBlock) -> starknet_core::types::ResourcePrice {
    let resource_price = &block.header().l1_gas_price;

    starknet_core::types::ResourcePrice {
        price_in_fri: resource_price.strk_l1_gas_price.into(),
        price_in_wei: resource_price.eth_l1_gas_price.into(),
    }
}

pub(crate) fn l1_data_gas_price(block: &DeoxysBlock) -> starknet_core::types::ResourcePrice {
    let resource_price = &block.header().l1_gas_price;

    starknet_core::types::ResourcePrice {
        price_in_fri: resource_price.strk_l1_data_gas_price.into(),
        price_in_wei: resource_price.eth_l1_data_gas_price.into(),
    }
}

pub(crate) fn l1_da_mode(block: &DeoxysBlock) -> starknet_core::types::L1DataAvailabilityMode {
    let l1_da_mode = &block.header().l1_da_mode;
    match l1_da_mode {
        L1DataAvailabilityMode::Calldata => starknet_core::types::L1DataAvailabilityMode::Calldata,
        L1DataAvailabilityMode::Blob => starknet_core::types::L1DataAvailabilityMode::Blob,
    }
}

pub(crate) fn starknet_version(block: &DeoxysBlock) -> String {
    block.header().protocol_version.to_string()
}
