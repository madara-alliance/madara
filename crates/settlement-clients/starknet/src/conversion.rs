use color_eyre::Result;
use starknet::core::types::Felt;

pub(crate) fn slice_slice_u8_to_vec_field(slices: &[[u8; 32]]) -> Vec<Felt> {
    slices.iter().map(slice_u8_to_field).collect()
}

pub(crate) fn slice_u8_to_field(slice: &[u8; 32]) -> Felt {
    Felt::from_bytes_be_slice(slice)
}

pub(crate) fn u64_from_felt(number: Felt) -> Result<u64> {
    let bytes = number.to_bytes_be();

    for x in &bytes[0..24] {
        if *x != 0 {
            return Err(color_eyre::Report::msg("byte should be zero, cannot convert to Felt"));
        }
    }
    Ok(u64::from_be_bytes(bytes[24..32].try_into()?))
}

#[test]
fn test_u64_from_from_felt_ok() {
    let number = 10.into();
    let converted = u64_from_felt(number);
    assert!(converted.expect("Failed to convert to u64") == 10u64, "Incorrect value conversion");
}

#[test]
fn test_u64_from_from_felt_panic() {
    let number = Felt::MAX;
    let number = u64_from_felt(number);
    match number {
        Ok(n) => tracing::debug!("Nonce value from get_nonce: {:?}", n),
        Err(e) => tracing::error!("Error getting nonce: {:?}", e),
    }
}
