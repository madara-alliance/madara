use starknet_types_core::felt::Felt;

#[derive(Debug, thiserror::Error)]
#[error("Felt is too big to convert to u64.")]
pub struct FeltToU64Error;

pub fn felt_to_u64(felt: &Felt) -> Result<u64, FeltToU64Error> {
    let digits = felt.to_be_digits();
    match (digits[0], digits[1], digits[2], digits[3]) {
        (0, 0, 0, d) => Ok(d),
        _ => Err(FeltToU64Error),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Felt is too big to convert to u128.")]
pub struct FeltToU128Error;

pub fn felt_to_u128(felt: &Felt) -> Result<u128, FeltToU128Error> {
    let digits = felt.to_be_digits();
    match (digits[0], digits[1], digits[2], digits[3]) {
        (0, 0, d1, d2) => Ok((d1 as u128) << 64 | d2 as u128),
        _ => Err(FeltToU128Error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_felt_to_u64() {
        assert_eq!(felt_to_u64(&Felt::ZERO).unwrap(), 0);
        assert_eq!(felt_to_u64(&Felt::ONE).unwrap(), 1);
        assert_eq!(felt_to_u64(&Felt::TWO).unwrap(), 2);
        assert_eq!(felt_to_u64(&Felt::THREE).unwrap(), 3);
        assert_eq!(felt_to_u64(&Felt::from(u32::MAX)).unwrap(), u32::MAX as u64);
        assert_eq!(felt_to_u64(&Felt::from(u64::MAX)).unwrap(), u64::MAX);
        assert!(felt_to_u64(&(Felt::from(u64::MAX) + Felt::ONE)).is_err());
        assert!(felt_to_u64(&Felt::MAX).is_err());
    }

    #[test]
    fn test_felt_to_u128() {
        assert_eq!(felt_to_u128(&Felt::ZERO).unwrap(), 0);
        assert_eq!(felt_to_u128(&Felt::ONE).unwrap(), 1);
        assert_eq!(felt_to_u128(&Felt::TWO).unwrap(), 2);
        assert_eq!(felt_to_u128(&Felt::THREE).unwrap(), 3);
        assert_eq!(felt_to_u128(&Felt::from(u64::MAX)).unwrap(), u64::MAX as u128);
        assert_eq!(felt_to_u128(&Felt::from(u128::MAX)).unwrap(), u128::MAX);
        assert!(felt_to_u128(&(Felt::from(u128::MAX) + Felt::ONE)).is_err());
        assert!(felt_to_u128(&Felt::MAX).is_err());
    }
}
