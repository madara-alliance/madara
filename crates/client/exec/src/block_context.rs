use blockifier::context::{BlockContext, FeeTokenAddresses};
use dp_block::Header;
use dp_convert::ToStarkFelt;
use starknet_types_core::felt::Felt;

pub const ETH_TOKEN_ADDR: Felt =
    Felt::from_hex_unchecked("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

pub const STRK_TOKEN_ADDR: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

pub fn block_context(block_header: &Header, chain_id: &Felt) -> Result<BlockContext, &'static str> {
    // safe unwrap because address is always valid and static
    let fee_token_address: FeeTokenAddresses = FeeTokenAddresses {
        strk_fee_token_address: STRK_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
        eth_fee_token_address: ETH_TOKEN_ADDR.to_stark_felt().try_into().unwrap(),
    };
    let chain_id = starknet_api::core::ChainId(
        String::from_utf8(chain_id.to_bytes_be().to_vec()).expect("Failed to convert chain id to string"),
    );

    block_header.into_block_context(fee_token_address, chain_id)
}
