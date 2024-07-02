use alloy::primitives::U256;

/// Converts a `&[Vec<u8>]` to `Vec<U256>`. Each inner slice is expected to be exactly 32 bytes long.
/// Pads with zeros if any inner slice is shorter than 32 bytes.
pub(crate) fn slice_slice_u8_to_vec_u256(slices: &[[u8; 32]]) -> Vec<U256> {
    slices.iter().map(|slice| slice_u8_to_u256(slice)).collect()
}

/// Converts a `&[u8]` to `U256`.
pub(crate) fn slice_u8_to_u256(slice: &[u8]) -> U256 {
    U256::try_from_be_slice(slice).expect("could not convert u8 slice to U256")
}
