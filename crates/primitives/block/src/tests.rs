use core::convert::TryFrom;

use blockifier::context::FeeTokenAddresses;
use mp_convert::ToFelt;
use starknet_api::core::ChainId;
use starknet_api::hash::StarkFelt;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

use super::{Header, StarknetVersion};

fn generate_dummy_header() -> Vec<Felt> {
    vec![
        Felt::ONE,  // block_number
        Felt::ONE,  // global_state_root
        Felt::ONE,  // sequencer_address
        Felt::ONE,  // block_timestamp
        Felt::ONE,  // transaction_count
        Felt::ONE,  // transaction_commitment
        Felt::ONE,  // event_count
        Felt::ONE,  // event_commitment
        Felt::ZERO, // placeholder
        Felt::ZERO, // placeholder
        Felt::ONE,  // parent_block_hash
    ]
}

#[test]
fn test_header_hash() {
    let hash = Pedersen::hash_array(&generate_dummy_header());

    let expected_hash = Felt::from_hex("0x001bef5f78bfd9122370a6bf9e3365b96362bef2bfd2b44b67707d8fbbf27bdc").unwrap();

    assert_eq!(hash, expected_hash);
}

#[test]
fn test_real_header_hash() {
    // Values taken from alpha-mainnet
    let block_number = 86000u32;
    let block_timestamp = 1687235884u32;
    let global_state_root =
        Felt::from_hex("0x006727a7aae8c38618a179aeebccd6302c67ad5f8528894d1dde794e9ae0bbfa").unwrap();
    let parent_block_hash =
        Felt::from_hex("0x045543088ce763aba7db8f6bfb33e33cc50af5c2ed5a26d38d5071c352a49c1d").unwrap();
    let sequencer_address =
        Felt::from_hex("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8").unwrap();
    let transaction_count = 197u32;
    let transaction_commitment =
        Felt::from_hex("0x70369cef825889dc005916dba67332b71f270b7af563d0433cee3342dda527d").unwrap();
    let event_count = 1430u32;
    let event_commitment = Felt::from_hex("0x2043ba1ef46882ce1dbb17b501fffa4b71f87f618e8f394e9605959d92efdf6").unwrap();
    let protocol_version = 0u32;

    let header: &[Felt] = &[
        block_number.into(),
        global_state_root,
        sequencer_address,
        block_timestamp.into(),
        transaction_count.into(),
        transaction_commitment,
        event_count.into(),
        event_commitment,
        protocol_version.into(),
        Felt::ZERO,
        parent_block_hash,
    ];

    let expected_hash = Felt::from_hex("0x001d126ca058c7e546d59cf4e10728e4b023ca0fb368e8abcabf0b5335f4487a").unwrap();
    let hash = Pedersen::hash_array(header);

    assert_eq!(hash, expected_hash);
}

#[test]
fn test_to_block_context() {
    let sequencer_address = Felt::from_hex_unchecked("0xFF");
    // Create a block header.
    let block_header = Header {
        block_number: 1,
        block_timestamp: 1,
        sequencer_address,
        protocol_version: StarknetVersion::STARKNET_VERSION_0_13_0,
        ..Default::default()
    };
    // Create a fee token address.
    let fee_token_addresses = FeeTokenAddresses {
        eth_fee_token_address: StarkFelt::try_from("0xAA").unwrap().try_into().unwrap(),
        strk_fee_token_address: StarkFelt::try_from("0xBB").unwrap().try_into().unwrap(),
    };
    // Create a chain id.
    let chain_id = ChainId("0x1".to_string());
    // Try to serialize the block header.
    let block_context = block_header.into_block_context(fee_token_addresses.clone(), chain_id).unwrap();
    // Check that the block context was serialized correctly.
    assert_eq!(block_context.block_info().block_number.0, 1);
    assert_eq!(block_context.block_info().block_timestamp.0, 1);
    assert_eq!(block_context.block_info().sequencer_address.to_felt(), sequencer_address);
    assert_eq!(
        &block_context.chain_info().fee_token_addresses.eth_fee_token_address,
        &fee_token_addresses.eth_fee_token_address
    );
    assert_eq!(
        &block_context.chain_info().fee_token_addresses.strk_fee_token_address,
        &fee_token_addresses.strk_fee_token_address
    );
}
