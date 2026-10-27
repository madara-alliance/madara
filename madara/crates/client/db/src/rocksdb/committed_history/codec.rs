//! Lossless storage codec only. This does not change the committed MPT or felt semantics.
use anyhow::{ensure, Result};
use starknet_types_core::felt::Felt;

fn put_var(mut x: u64, out: &mut Vec<u8>) {
    while x >= 128 {
        out.push((x as u8 & 127) | 128);
        x >>= 7;
    }
    out.push(x as u8);
}
fn var(input: &mut &[u8]) -> Result<u64> {
    let mut x = 0;
    for i in 0..10 {
        ensure!(!input.is_empty(), "truncated varint");
        let b = input[0];
        *input = &input[1..];
        ensure!(i != 9 || b <= 1, "varint overflow");
        x |= u64::from(b & 127) << (7 * i);
        if b < 128 {
            ensure!(i == 0 || b != 0, "noncanonical varint");
            return Ok(x);
        }
    }
    anyhow::bail!("varint overflow")
}
fn small(f: Felt) -> Option<u64> {
    let b = f.to_bytes_be();
    b[..24].iter().all(|b| *b == 0).then(|| u64::from_be_bytes(b[24..].try_into().expect("eight-byte suffix")))
}
fn absolute(f: Felt) -> Vec<u8> {
    let b = f.to_bytes_be();
    let start = b.iter().position(|b| *b != 0).unwrap_or(32);
    let mut out = vec![0, (32 - start) as u8];
    out.extend(&b[start..]);
    out
}
fn encode_value(old: Felt, new: Felt) -> Vec<u8> {
    let abs = absolute(new);
    if let (Some(a), Some(b)) = (small(old), small(new)) {
        if let Ok(d) = i64::try_from(i128::from(b) - i128::from(a)) {
            let zz = ((d as u64) << 1) ^ ((d >> 63) as u64);
            if let Some(token) = zz.checked_add(1) {
                let mut out = vec![];
                put_var(token, &mut out);
                if out.len() < abs.len() {
                    return out;
                }
            }
        }
    }
    abs
}
pub(super) fn felt(b: &[u8]) -> Result<Felt> {
    let bytes: [u8; 32] = b.try_into()?;
    let f = Felt::from_bytes_be(&bytes);
    ensure!(f.to_bytes_be() == bytes, "noncanonical felt");
    Ok(f)
}
fn decode_value(input: &mut &[u8], old: Felt) -> Result<Felt> {
    let before = *input;
    let token = var(input)?;
    let value = if token == 0 {
        ensure!(!input.is_empty(), "missing absolute length");
        let len = input[0] as usize;
        *input = &input[1..];
        ensure!(len <= 32 && input.len() >= len, "absolute length");
        ensure!(len == 0 || input[0] != 0, "leading zero");
        let mut b = [0; 32];
        b[32 - len..].copy_from_slice(&input[..len]);
        *input = &input[len..];
        felt(&b)?
    } else {
        let a = small(old).ok_or_else(|| anyhow::anyhow!("delta on wide base"))?;
        let zz = token - 1;
        let d = ((zz >> 1) as i64) ^ (-((zz & 1) as i64));
        let b = u64::try_from(i128::from(a) + i128::from(d))?;
        Felt::from(b)
    };
    ensure!(encode_value(old, value) == before[..before.len() - input.len()], "noncanonical value encoding");
    Ok(value)
}
/// Ordered dictionary indices followed by lossless price deltas. Wide felts use absolute values.
pub(super) fn encode(changes: impl IntoIterator<Item = (usize, Felt, Felt)>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = 0;
    for (index, old, new) in changes {
        put_var((index + 1 - previous) as u64, &mut out);
        previous = index + 1;
        out.extend(encode_value(old, new));
    }
    out
}

pub(super) fn apply(mut input: &[u8], values: &mut [Felt]) -> Result<usize> {
    let mut previous = 0usize;
    let mut count = 0;
    while !input.is_empty() {
        let gap = usize::try_from(var(&mut input)?)?;
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
    #[test]
    fn lossless_boundaries_and_rejections() -> Result<()> {
        let values =
            [Felt::ZERO, Felt::ONE, Felt::from(u64::MAX), Felt::from(128u64), Felt::MAX, Felt::MAX - Felt::ONE];
        for old in values {
            for new in values {
                let bytes = encode_value(old, new);
                assert_eq!(decode_value(&mut bytes.as_slice(), old)?, new);
                for end in 0..bytes.len() {
                    assert!(decode_value(&mut &bytes[..end], old).is_err());
                }
                let encoded = encode([(1, old, new)]);
                let mut decoded = [Felt::TWO, old];
                assert_eq!(apply(&encoded, &mut decoded)?, 1);
                assert_eq!(decoded, [Felt::TWO, new]);
            }
        }
        assert!(var(&mut &[255u8; 10][..]).is_err());
        assert!(var(&mut &[128u8, 0][..]).is_err());
        assert!(decode_value(&mut &[3u8][..], Felt::from(u64::MAX)).is_err());
        assert!(decode_value(&mut &[2u8][..], Felt::ZERO).is_err());
        assert!(decode_value(&mut &[1u8][..], Felt::MAX).is_err());
        assert!(decode_value(&mut &[0u8, 1, 0][..], Felt::ZERO).is_err());
        assert!(decode_value(&mut &[0u8, 32].into_iter().chain([255; 32]).collect::<Vec<_>>().as_slice(), Felt::ZERO)
            .is_err());
        assert!(apply(&[0, 1], &mut [Felt::ZERO]).is_err());
        assert!(apply(&[2, 1], &mut [Felt::ZERO]).is_err());
        Ok(())
    }
    #[test]
    fn changing_prices_and_wide_values_roundtrip() -> Result<()> {
        let mut old = Felt::from(1_000_000u64);
        let mut x = 17u64;
        for i in 0..10000 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = if i % 10 == 0 { Felt::from(x) } else { old + Felt::from(x % 100) };
            let bytes = encode_value(old, value);
            assert_eq!(decode_value(&mut bytes.as_slice(), old)?, value);
            old = value;
        }
        assert_eq!(encode_value(Felt::from(1_000_000), Felt::from(1_000_001)).len(), 1);
        assert_eq!(encode_value(Felt::ZERO, Felt::MAX).len(), 34);
        Ok(())
    }
}
