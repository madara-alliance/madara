use alloy::primitives::U256;
use primitive_types::H160;
use starknet_types_core::felt::Felt;

#[derive(Debug, thiserror::Error)]
#[error("Felt is too big to convert to H160.")]
pub struct FeltToH160Error;

pub fn felt_to_h160(felt: &Felt) -> Result<H160, FeltToH160Error> {
    const MAX_H160: Felt = Felt::from_hex_unchecked("0xffffffffffffffffffffffffffffffffffffffff");

    if felt > &MAX_H160 {
        return Err(FeltToH160Error);
    }

    let bytes = felt.to_bytes_be();

    let mut h160_bytes = [0u8; 20];
    h160_bytes.copy_from_slice(&bytes[12..]);
    Ok(H160::from(h160_bytes))
}

pub fn felt_to_u256(felt: Felt) -> Result<U256, String> {
    let bytes = felt.to_bytes_be();
    if bytes.len() > 32 {
        return Err("Felt value too large for U256".into());
    }
    Ok(U256::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    #[test]
    fn test_felt_tu_h160() {
        const MAX_H160: [u8; 20] = [0xff; 20];
        assert_eq!(felt_to_h160(&Felt::ZERO).unwrap(), H160::zero());
        assert_eq!(felt_to_h160(&Felt::ONE).unwrap(), H160::from_low_u64_be(1));
        assert_eq!(felt_to_h160(&Felt::TWO).unwrap(), H160::from_low_u64_be(2));
        assert_eq!(felt_to_h160(&Felt::THREE).unwrap(), H160::from_low_u64_be(3));
        assert_eq!(felt_to_h160(&Felt::from(u64::MAX)).unwrap(), H160::from_low_u64_be(u64::MAX));
        assert_eq!(felt_to_h160(&Felt::from_bytes_be_slice(&MAX_H160)).unwrap(), H160::from_slice(&MAX_H160));
        assert_matches!(felt_to_h160(&(Felt::from_bytes_be_slice(&MAX_H160) + Felt::ONE)), Err(FeltToH160Error));
        assert_matches!(felt_to_h160(&Felt::MAX), Err(FeltToH160Error));
    }
}
