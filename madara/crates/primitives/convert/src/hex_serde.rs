use std::fmt;

use serde::de::Visitor;
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

pub struct U128AsHexOrNumber;

impl SerializeAs<u128> for U128AsHexOrNumber {
    fn serialize_as<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        U128AsHex::serialize_as(value, serializer)
    }
}

impl<'de> DeserializeAs<'de, u128> for U128AsHexOrNumber {
    fn deserialize_as<D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct U128HexOrNumberVisitor;

        impl<'de> Visitor<'de> for U128HexOrNumberVisitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a u128 encoded as a hex string, decimal string, or JSON integer")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(u128::from(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                u128::try_from(value).map_err(E::custom)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value.starts_with("0x") || value.starts_with("0X") {
                    u128::from_str_radix(value.trim_start_matches("0x").trim_start_matches("0X"), 16).map_err(E::custom)
                } else {
                    value.parse::<u128>().map_err(E::custom)
                }
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(U128HexOrNumberVisitor)
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
    use serde_with::serde_as;

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

    #[test]
    fn test_u128_as_hex_or_number_deserializes_hex_and_number() {
        #[serde_as]
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde_as(as = "U128AsHexOrNumber")]
            value: u128,
        }

        let hex = serde_json::from_str::<Wrapper>(r#"{"value":"0x7530"}"#).unwrap();
        assert_eq!(hex.value, 30_000);

        let number = serde_json::from_str::<Wrapper>(r#"{"value":30000}"#).unwrap();
        assert_eq!(number.value, 30_000);
    }
}
