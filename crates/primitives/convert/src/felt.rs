use starknet_core::types::Felt;

pub fn felt_to_u64(felt: &Felt) -> Result<u64, ()> {
    let digits = felt.to_be_digits();
    if digits[0] != 0 || digits[1] != 0 || digits[2] != 0 {
        return Err(());
    }
    Ok(digits[3])
}

pub fn felt_to_u128(felt: &Felt) -> Result<u128, ()> {
    let digits = felt.to_be_digits();
    if digits[0] != 0 || digits[1] != 0 {
        return Err(());
    }
    Ok((digits[2] as u128) << 64 | digits[3] as u128)
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
        assert_eq!(felt_to_u64(&(Felt::from(u64::MAX) + Felt::ONE)).is_err(), true);
        assert_eq!(felt_to_u64(&Felt::MAX).is_err(), true);
    }

    #[test]
    fn test_felt_to_u128() {
        assert_eq!(felt_to_u128(&Felt::ZERO).unwrap(), 0);
        assert_eq!(felt_to_u128(&Felt::ONE).unwrap(), 1);
        assert_eq!(felt_to_u128(&Felt::TWO).unwrap(), 2);
        assert_eq!(felt_to_u128(&Felt::THREE).unwrap(), 3);
        assert_eq!(felt_to_u128(&Felt::from(u64::MAX)).unwrap(), u64::MAX as u128);
        assert_eq!(felt_to_u128(&Felt::from(u128::MAX)).unwrap(), u128::MAX);
        assert_eq!(felt_to_u128(&(Felt::from(u128::MAX) + Felt::ONE)).is_err(), true);
        assert_eq!(felt_to_u128(&Felt::MAX).is_err(), true);
    }
}
