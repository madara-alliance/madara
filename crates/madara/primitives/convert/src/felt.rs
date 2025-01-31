use primitive_types::H160;
use starknet_types_core::felt::Felt;
use std::cmp::Ordering;

#[derive(Debug, thiserror::Error)]
#[error("Malformated field element.")]
pub struct MalformatedFelt;

pub trait FeltExt {
    fn from_slice_be_checked(slice: &[u8]) -> Result<Felt, MalformatedFelt>;
    fn from_bytes_checked(slice: &[u8; 32]) -> Result<Felt, MalformatedFelt>;

    fn slice_be_len(&self) -> usize;
    fn to_h160(&self) -> Result<H160, FeltToH160Error>;
}

impl FeltExt for Felt {
    fn from_slice_be_checked(slice: &[u8]) -> Result<Felt, MalformatedFelt> {
        if slice.len() > 32 {
            return Err(MalformatedFelt);
        }

        let mut unpacked = [0; 32];
        for i in 0..slice.len() {
            unpacked[32 - slice.len() + i] = slice[i]
        }

        Felt::from_bytes_checked(&unpacked)
    }

    fn from_bytes_checked(b: &[u8; 32]) -> Result<Felt, MalformatedFelt> {
        let limbs = [
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            u64::from_be_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
            u64::from_be_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]),
            u64::from_be_bytes([b[24], b[25], b[26], b[27], b[28], b[29], b[30], b[31]]),
        ];
        // Check if it overflows the modulus.

        // p=2^251 + 17*2^192 + 1
        const MODULUS_U64: [u64; 4] = [576460752303423505u64, 0, 0, 1];

        for i in 0..4 {
            match u64::cmp(&limbs[i], &MODULUS_U64[i]) {
                Ordering::Less => break,
                Ordering::Equal if i == 3 => return Err(MalformatedFelt),
                Ordering::Equal => continue,
                Ordering::Greater => return Err(MalformatedFelt),
            }
        }

        Ok(Felt::from_bytes_be(b))
    }

    fn slice_be_len(&self) -> usize {
        let bytes = self.to_bytes_be();
        let mut len = 32;
        while len > 0 && bytes[32 - len] == 0 {
            len -= 1;
        }
        len
    }

    fn to_h160(&self) -> Result<H160, FeltToH160Error> {
        felt_to_h160(self)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Felt is too big to convert to H160.")]
pub struct FeltToH160Error;

fn felt_to_h160(felt: &Felt) -> Result<H160, FeltToH160Error> {
    const MAX_H160: Felt = Felt::from_hex_unchecked("0xffffffffffffffffffffffffffffffffffffffff");

    if felt > &MAX_H160 {
        return Err(FeltToH160Error);
    }

    let bytes = felt.to_bytes_be();

    let mut h160_bytes = [0u8; 20];
    h160_bytes.copy_from_slice(&bytes[12..]);
    Ok(H160::from(h160_bytes))
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
