use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct L1TxnHash(pub [u8; 32]);

impl Serialize for L1TxnHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = String::with_capacity(66);
        s.push_str("0x");
        for byte in &self.0 {
            s.push_str(&format!("{:02x}", byte));
        }
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for L1TxnHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = Deserialize::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() > 64 {
            return Err(serde::de::Error::custom("Invalid length for L1TxnHash"));
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in s.as_bytes().chunks(2).enumerate() {
            let byte_str = std::str::from_utf8(byte).map_err(serde::de::Error::custom)?;
            bytes[i] = u8::from_str_radix(byte_str, 16).map_err(serde::de::Error::custom)?;
        }
        Ok(L1TxnHash(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    const TEST_HASH: L1TxnHash = L1TxnHash([
        0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34,
        0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef,
    ]);

    const TEST_HASH_STR: &str = "\"0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\"";

    #[test]
    fn test_l1_transaction_hash_serialization() {
        let hash = L1TxnHash([0u8; 32]);
        let serialized = serde_json::to_string(&hash).unwrap();
        assert_eq!(serialized, "\"0x0000000000000000000000000000000000000000000000000000000000000000\"");
        let deserialized: L1TxnHash = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, hash);
    }

    #[test]
    fn test_l1_transaction_hash_deserialization() {
        let deserialized: L1TxnHash = serde_json::from_str(TEST_HASH_STR).unwrap();
        assert_eq!(deserialized, TEST_HASH);
    }

    #[test]
    fn test_l1_transaction_hash_deserialization_no_prefix() {
        let deserialized: L1TxnHash =
            serde_json::from_str("\"1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\"").unwrap();
        assert_eq!(deserialized, TEST_HASH);
    }

    #[test]
    fn test_l1_transaction_hash_deserialization_small() {
        let deserialized: L1TxnHash =
            serde_json::from_str("\"0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\"").unwrap();
        assert_eq!(deserialized, TEST_HASH);
    }

    #[test]
    fn test_l1_transaction_hash_deserialization_invalid_length() {
        let result: Result<L1TxnHash, _> = serde_json::from_str("\"0x123\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_l1_transaction_hash_deserialization_invalid_hex() {
        let result: Result<L1TxnHash, _> = serde_json::from_str(
            "\"
0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdeg\"",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_l1_transaction_hash_serialization_deserialization() {
        let original = TEST_HASH;
        let serialized = serde_json::to_string(&original).unwrap();
        assert_eq!(serialized, TEST_HASH_STR);
        let deserialized: L1TxnHash = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, original);
    }
}
