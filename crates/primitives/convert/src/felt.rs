use std::cmp::Ordering;

use starknet_types_core::felt::Felt;

#[derive(Debug, thiserror::Error)]
#[error("Malformated field element.")]
pub struct MalformatedFelt;

pub trait FeltExt {
    fn from_slice_be_checked(slice: &[u8]) -> Result<Felt, MalformatedFelt>;
    fn from_bytes_checked(slice: &[u8; 32]) -> Result<Felt, MalformatedFelt>;

    fn slice_be_len(&self) -> usize;
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
}

#[derive(Debug, thiserror::Error)]
#[error("Felt is too big to convert to u64.")]
pub struct FeltToU32Error;

pub fn felt_to_u32(felt: &Felt) -> Result<u32, FeltToU64Error> {
    let digits = felt.to_be_digits();
    match (digits[0], digits[1], digits[2], digits[3]) {
        (0, 0, 0, d) => d.try_into().map_err(|_| FeltToU64Error),
        _ => Err(FeltToU64Error),
    }
}

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
    use assert_matches::assert_matches;

    #[test]
    fn test_felt_to_u64() {
        assert_eq!(felt_to_u64(&Felt::ZERO).unwrap(), 0);
        assert_eq!(felt_to_u64(&Felt::ONE).unwrap(), 1);
        assert_eq!(felt_to_u64(&Felt::TWO).unwrap(), 2);
        assert_eq!(felt_to_u64(&Felt::THREE).unwrap(), 3);
        assert_eq!(felt_to_u64(&Felt::from(u32::MAX)).unwrap(), u32::MAX as u64);
        assert_eq!(felt_to_u64(&Felt::from(u64::MAX)).unwrap(), u64::MAX);
        assert_matches!(felt_to_u64(&(Felt::from(u64::MAX) + Felt::ONE)), Err(FeltToU64Error));
        assert_matches!(felt_to_u64(&Felt::MAX), Err(FeltToU64Error));
    }

    #[test]
    fn test_felt_to_u128() {
        assert_eq!(felt_to_u128(&Felt::ZERO).unwrap(), 0);
        assert_eq!(felt_to_u128(&Felt::ONE).unwrap(), 1);
        assert_eq!(felt_to_u128(&Felt::TWO).unwrap(), 2);
        assert_eq!(felt_to_u128(&Felt::THREE).unwrap(), 3);
        assert_eq!(felt_to_u128(&Felt::from(u64::MAX)).unwrap(), u64::MAX as u128);
        assert_eq!(felt_to_u128(&Felt::from(u128::MAX)).unwrap(), u128::MAX);
        assert_matches!(felt_to_u128(&(Felt::from(u128::MAX) + Felt::ONE)), Err(FeltToU128Error));
        assert_matches!(felt_to_u128(&Felt::MAX), Err(FeltToU128Error));
    }

    #[test]
    fn test_felt_to_slice_be() {
        let to_vec_be = |f: Felt| {
            let bytes = f.to_bytes_be();
            let slice_be_len = f.slice_be_len();
            bytes[32 - slice_be_len..32].to_vec()
        };

        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x0")), Vec::<u8>::new());
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x1")), vec![1]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x2")), vec![2]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x3")), vec![3]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x10")), vec![16]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x100")), vec![1, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x1001")), vec![16, 1]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x10000")), vec![1, 0, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x1000000")), vec![1, 0, 0, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x100000000")), vec![1, 0, 0, 0, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x10000000000")), vec![1, 0, 0, 0, 0, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x1000000000000")), vec![1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(to_vec_be(Felt::from_hex_unchecked("0x100000000000000")), vec![1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            to_vec_be(Felt::from_hex_unchecked("0x10000000000000000000000000000")),
            vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            to_vec_be(Felt::from_hex_unchecked("0x00EC7BAE44AA0E2532DE25874FF8090F5416CD8974D05EB3FD7A62251A0AEFCA")),
            vec![
                236, 123, 174, 68, 170, 14, 37, 50, 222, 37, 135, 79, 248, 9, 15, 84, 22, 205, 137, 116, 208, 94, 179,
                253, 122, 98, 37, 26, 10, 239, 202
            ]
        );
        assert_eq!(
            to_vec_be(Felt::from_hex_unchecked("0x03EBF191E97EB162CC34B41F74A24D3638072DF3BFC7408CBFF41AAA16FF89E0")),
            vec![
                3, 235, 241, 145, 233, 126, 177, 98, 204, 52, 180, 31, 116, 162, 77, 54, 56, 7, 45, 243, 191, 199, 64,
                140, 191, 244, 26, 170, 22, 255, 137, 224
            ]
        );
    }
}
