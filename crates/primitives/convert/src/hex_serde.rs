use serde::{Deserialize, Deserializer, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

pub struct U64AsHex;

impl SerializeAs<u64> for U64AsHex {
    fn serialize_as<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{:x}", value))
    }
}

impl<'de> DeserializeAs<'de, u64> for U64AsHex {
    fn deserialize_as<D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(serde::de::Error::custom)
    }
}

pub fn hex_str_to_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
}

pub fn u64_to_hex_string(n: u64) -> String {
    format!("0x{:x}", n)
}

pub struct U128AsHex;

impl SerializeAs<u128> for U128AsHex {
    fn serialize_as<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{:x}", value))
    }
}

impl<'de> DeserializeAs<'de, u128> for U128AsHex {
    fn deserialize_as<D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        u128::from_str_radix(s.trim_start_matches("0x"), 16).map_err(serde::de::Error::custom)
    }
}

pub fn hex_str_to_u128(s: &str) -> Result<u128, std::num::ParseIntError> {
    u128::from_str_radix(s.trim_start_matches("0x"), 16)
}

pub fn u128_to_hex_string(n: u128) -> String {
    format!("0x{:x}", n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u64_as_hex() {
        let n = 0x1234567890abcdef;
        let s = u64_to_hex_string(n);
        assert_eq!(s, "0x1234567890abcdef");
        let m = hex_str_to_u64(&s).unwrap();
        assert_eq!(m, n);
    }

    #[test]
    fn test_u128_as_hex() {
        let n = 0x1234567890abcdef1234567890abcdef;
        let s = u128_to_hex_string(n);
        assert_eq!(s, "0x1234567890abcdef1234567890abcdef");
        let m = hex_str_to_u128(&s).unwrap();
        assert_eq!(m, n);
    }
}
