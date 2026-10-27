//! Lossless value-history storage; hashes and witnesses always use the original felts.
//!
//! A sorted key dictionary is stored separately. Each changed value is encoded as a positive
//! varint gap from the previous dictionary index, followed by one canonical value token:
//! - token 0: one length byte and the shortest big-endian absolute felt (zero has length 0);
//! - token > 0: zigzag(signed delta) + 1, only when both values fit u64 and this is shorter.
//!
//! Ties use absolute encoding. Values are never rounded, scaled or narrowed.
//! The archive compresses the resulting byte stream with Zstd; this module only packs it.
//! Decoding rejects noncanonical integers, duplicate/out-of-range indices and arithmetic overflow.
use anyhow::{ensure, Result};
use starknet_types_core::felt::Felt;

const FELT_BYTES: usize = 32;
const U64_SUFFIX: usize = FELT_BYTES - size_of::<u64>();
const VARINT_PAYLOAD_BITS: usize = 7;
const VARINT_CONTINUATION: u8 = 0x80;
const VARINT_PAYLOAD_MASK: u8 = VARINT_CONTINUATION - 1;
const MAX_U64_VARINT_BYTES: usize = 10;

/// Write unsigned LEB128, using the shortest representation.
fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= u64::from(VARINT_CONTINUATION) {
        out.push((value as u8 & VARINT_PAYLOAD_MASK) | VARINT_CONTINUATION);
        value >>= VARINT_PAYLOAD_BITS;
    }
    out.push(value as u8);
}
/// Consume one canonical u64 LEB128; malformed persisted bytes fail closed.
fn decode_varint(input: &mut &[u8]) -> Result<u64> {
    let mut value = 0;
    for byte_index in 0..MAX_U64_VARINT_BYTES {
        ensure!(!input.is_empty(), "truncated varint");
        let byte = input[0];
        *input = &input[1..];
        ensure!(byte_index != MAX_U64_VARINT_BYTES - 1 || byte <= 1, "varint overflow");
        value |= u64::from(byte & VARINT_PAYLOAD_MASK) << (VARINT_PAYLOAD_BITS * byte_index);
        if byte < VARINT_CONTINUATION {
            ensure!(byte_index == 0 || byte != 0, "noncanonical varint");
            return Ok(value);
        }
    }
    anyhow::bail!("varint overflow")
}
/// Use delta encoding only when the entire felt fits u64; high bytes must all be zero.
fn as_u64(value: Felt) -> Option<u64> {
    let bytes = value.to_bytes_be();
    bytes[..U64_SUFFIX]
        .iter()
        .all(|bytes| *bytes == 0)
        .then(|| u64::from_be_bytes(bytes[U64_SUFFIX..].try_into().expect("eight-byte suffix")))
}
/// Encode token zero, significant-byte length, then the shortest big-endian felt bytes.
fn encode_absolute(value: Felt) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let start = bytes.iter().position(|bytes| *bytes != 0).unwrap_or(FELT_BYTES);
    let mut out = vec![0, (FELT_BYTES - start) as u8];
    out.extend(&bytes[start..]);
    out
}
/// Choose the shorter lossless representation; no application-specific value width is assumed.
fn encode_value(old: Felt, new: Felt) -> Vec<u8> {
    let absolute_bytes = encode_absolute(new);
    if let (Some(previous), Some(next)) = (as_u64(old), as_u64(new)) {
        if let Ok(delta) = i64::try_from(i128::from(next) - i128::from(previous)) {
            let zigzag = ((delta as u64) << 1) ^ ((delta >> 63) as u64);
            if let Some(token) = zigzag.checked_add(1) {
                let mut out = vec![];
                encode_varint(token, &mut out);
                if out.len() < absolute_bytes.len() {
                    return out;
                }
            }
        }
    }
    absolute_bytes
}
/// Decode exactly 32 canonical bytes without allowing modular reduction.
pub(super) fn felt(encoded: &[u8]) -> Result<Felt> {
    let bytes: [u8; FELT_BYTES] = encoded.try_into()?;
    let value = Felt::from_bytes_be(&bytes);
    ensure!(value.to_bytes_be() == bytes, "noncanonical felt");
    Ok(value)
}
/// Decode one token and re-encode it to reject alternate encodings of the same value.
fn decode_value(input: &mut &[u8], old: Felt) -> Result<Felt> {
    let before = *input;
    let token = decode_varint(input)?;
    let value = if token == 0 {
        ensure!(!input.is_empty(), "missing absolute length");
        let len = input[0] as usize;
        *input = &input[1..];
        ensure!(len <= FELT_BYTES && input.len() >= len, "absolute length");
        ensure!(len == 0 || input[0] != 0, "leading zero");
        let mut decoded = [0; FELT_BYTES];
        decoded[FELT_BYTES - len..].copy_from_slice(&input[..len]);
        *input = &input[len..];
        felt(&decoded)?
    } else {
        let previous = as_u64(old).ok_or_else(|| anyhow::anyhow!("delta on wide base"))?;
        let zigzag = token - 1;
        let delta = ((zigzag >> 1) as i64) ^ (-((zigzag & 1) as i64));
        let decoded = u64::try_from(i128::from(previous) + i128::from(delta))?;
        Felt::from(decoded)
    };
    ensure!(encode_value(old, value) == before[..before.len() - input.len()], "noncanonical value encoding");
    Ok(value)
}
/// Ordered dictionary indices followed by lossless value deltas. Wide felts use absolute values.
pub(super) fn encode(changes: impl IntoIterator<Item = (usize, Felt, Felt)>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = 0;
    for (index, old, new) in changes {
        encode_varint((index + 1 - previous) as u64, &mut out);
        previous = index + 1;
        out.extend(encode_value(old, new));
    }
    out
}

/// Apply strictly increasing dictionary indices in place; return the number of changed slots.
/// The caller discards this reconstruction on any decode error.
pub(super) fn apply(mut input: &[u8], values: &mut [Felt]) -> Result<usize> {
    let mut previous = 0usize;
    let mut count = 0;
    while !input.is_empty() {
        let gap = usize::try_from(decode_varint(&mut input)?)?;
        ensure!(gap > 0, "Duplicate dictionary index");
        previous = previous.checked_add(gap).ok_or_else(|| anyhow::anyhow!("Index overflow"))?;
        ensure!(previous <= values.len(), "Dictionary index out of bounds");
        values[previous - 1] = decode_value(&mut input, values[previous - 1])?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Cross-product coverage: every boundary acts as both a base and a replacement.
    #[rstest::rstest]
    fn value_boundaries_roundtrip_and_reject_every_truncation(
        #[values(Felt::ZERO, Felt::ONE, Felt::from(u64::MAX), Felt::from(128u64), Felt::MAX, Felt::MAX - Felt::ONE)]
        old: Felt,
        #[values(Felt::ZERO, Felt::ONE, Felt::from(u64::MAX), Felt::from(128u64), Felt::MAX, Felt::MAX - Felt::ONE)]
        new: Felt,
    ) -> Result<()> {
        let bytes = encode_value(old, new);
        assert_eq!(decode_value(&mut bytes.as_slice(), old)?, new);
        for end in 0..bytes.len() {
            assert!(decode_value(&mut &bytes[..end], old).is_err(), "Accepted truncation at {end}");
        }
        let mut values = [Felt::TWO, old];
        assert_eq!(apply(&encode([(1, old, new)]), &mut values)?, 1);
        assert_eq!(values, [Felt::TWO, new]);
        Ok(())
    }

    #[rstest::rstest]
    #[case::overflow(vec![255; 10])]
    #[case::overlong(vec![128, 0])]
    fn malformed_varints_are_rejected(#[case] bytes: Vec<u8>) {
        assert!(decode_varint(&mut bytes.as_slice()).is_err());
    }

    #[rstest::rstest]
    #[case::positive_overflow(Felt::from(u64::MAX), vec![3])]
    #[case::negative_underflow(Felt::ZERO, vec![2])]
    #[case::wide_base(Felt::MAX, vec![1])]
    #[case::leading_zero(Felt::ZERO, vec![0, 1, 0])]
    #[case::noncanonical_felt(Felt::ZERO, [vec![0, 32], vec![255; 32]].concat())]
    fn invalid_value_tokens_are_rejected(#[case] old: Felt, #[case] bytes: Vec<u8>) {
        assert!(decode_value(&mut bytes.as_slice(), old).is_err());
    }

    #[rstest::rstest]
    #[case::duplicate_index([0, 1])]
    #[case::out_of_bounds([2, 1])]
    fn invalid_dictionary_indices_are_rejected(#[case] bytes: [u8; 2]) {
        assert!(apply(&bytes, &mut [Felt::ZERO]).is_err());
    }

    #[test]
    fn dictionary_gaps_and_signed_deltas_have_fixed_encoding() -> Result<()> {
        // Independent hand calculation: gaps 1/2, zigzag(+1)+1=3, zigzag(-1)+1=2.
        let bytes = encode([(0, Felt::from(100), Felt::from(101)), (2, Felt::from(200), Felt::from(199))]);
        assert_eq!(bytes, [1, 3, 2, 2]);
        let mut values = [Felt::from(100), Felt::MAX, Felt::from(200)];
        assert_eq!(apply(&bytes, &mut values)?, 2);
        assert_eq!(values, [Felt::from(101), Felt::MAX, Felt::from(199)]);
        Ok(())
    }
    #[test]
    fn changing_values_and_wide_values_roundtrip() -> Result<()> {
        let mut old = Felt::from(1_000_000u64);
        let mut random_state = 17u64;
        for i in 0..10000 {
            random_state = random_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = if i % 10 == 0 { Felt::from(random_state) } else { old + Felt::from(random_state % 100) };
            let bytes = encode_value(old, value);
            assert_eq!(decode_value(&mut bytes.as_slice(), old)?, value);
            old = value;
        }
        assert_eq!(encode_value(Felt::from(1_000_000), Felt::from(1_000_001)).len(), 1);
        assert_eq!(encode_value(Felt::ZERO, Felt::MAX).len(), 34);
        Ok(())
    }
}
