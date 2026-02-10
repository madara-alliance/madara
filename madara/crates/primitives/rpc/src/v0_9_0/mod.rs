//! v0.9.0 of the API.
mod starknet_api_openrpc;
mod starknet_trace_api_openrpc;
mod starknet_ws_api;

pub use self::starknet_api_openrpc::*;
pub use self::starknet_trace_api_openrpc::*;
pub use self::starknet_ws_api::*;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum BlockId {
    /// The tag of the block.
    Tag(BlockTag),
    /// The hash of the block.
    Hash(BlockHash),
    /// The height of the block.
    Number(BlockNumber),
}

impl serde::Serialize for BlockId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            BlockId::Tag(tag) => tag.serialize(serializer),
            BlockId::Hash(block_hash) => BlockHashHelper { block_hash: *block_hash }.serialize(serializer),
            BlockId::Number(block_number) => BlockNumberHelper { block_number: *block_number }.serialize(serializer),
        }
    }
}

impl<'de> serde::Deserialize<'de> for BlockId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let helper = BlockIdHelper::deserialize(deserializer)?;
        match helper {
            BlockIdHelper::Tag(tag) => Ok(BlockId::Tag(tag)),
            BlockIdHelper::Hash(helper) => Ok(BlockId::Hash(helper.block_hash)),
            BlockIdHelper::Number(helper) => Ok(BlockId::Number(helper.block_number)),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum BlockIdHelper {
    Tag(BlockTag),
    Hash(BlockHashHelper),
    Number(BlockNumberHelper),
}

#[test]
fn block_id_from_hash() {
    pub use starknet_types_core::felt::Felt;

    let s = "{\"block_hash\":\"0x123\"}";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Hash(Felt::from_hex("0x123").unwrap()));
}

#[test]
fn block_id_from_number() {
    let s = "{\"block_number\":123}";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Number(123));
}

#[test]
fn block_id_from_latest() {
    let s = "\"latest\"";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Tag(BlockTag::Latest));
}

#[test]
fn block_id_from_pre_confirmed() {
    let s = "\"pre_confirmed\"";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Tag(BlockTag::PreConfirmed));
}

#[test]
fn block_id_from_l1_accepted() {
    let s = "\"l1_accepted\"";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Tag(BlockTag::L1Accepted));
}

#[cfg(test)]
#[test]
fn block_id_to_hash() {
    pub use starknet_types_core::felt::Felt;

    let block_id = BlockId::Hash(Felt::from_hex("0x123").unwrap());
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "{\"block_hash\":\"0x123\"}");
}

#[cfg(test)]
#[test]
fn block_id_to_number() {
    let block_id = BlockId::Number(123);
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "{\"block_number\":123}");
}

#[cfg(test)]
#[test]
fn block_id_to_latest() {
    let block_id = BlockId::Tag(BlockTag::Latest);
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "\"latest\"");
}

#[cfg(test)]
#[test]
fn block_id_to_pre_confirmed() {
    let block_id = BlockId::Tag(BlockTag::PreConfirmed);
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "\"pre_confirmed\"");
}

#[cfg(test)]
#[test]
fn block_id_to_l1_accepted() {
    let block_id = BlockId::Tag(BlockTag::L1Accepted);
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "\"l1_accepted\"");
}

/// The hash of an Ethereum transaction.
///
/// Spec: `L1_TXN_HASH` is `NUM_AS_HEX` (`^0x[a-fA-F0-9]+$`). We accept 1..=64 hex digits
/// after the `0x` prefix and left-pad to 32 bytes. We always serialize to a 32-byte
/// canonical form (`0x` + 64 lowercase hex digits).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct L1TxnHash(pub [u8; 32]);

impl Serialize for L1TxnHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = String::with_capacity(2 + 64);
        s.push_str("0x");
        for b in self.0 {
            use core::fmt::Write as _;
            write!(&mut s, "{:02x}", b).expect("writing into String cannot fail");
        }
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for L1TxnHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = L1TxnHash;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("a 0x-prefixed hex string with 1 to 64 hex digits")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let Some(hex) = v.strip_prefix("0x") else {
                    return Err(E::custom("expected a 0x-prefixed hex string"));
                };
                if hex.is_empty() {
                    return Err(E::custom("expected at least 1 hex digit after 0x"));
                }
                if hex.len() > 64 {
                    return Err(E::custom("expected at most 64 hex digits after 0x"));
                }

                // Parse a 32-byte big-endian hash from a 0x-prefixed hex string.
                //
                // - We accept 1..=64 hex digits (spec: NUM_AS_HEX), and left-pad with zeros.
                // - Odd-length inputs are allowed: the first nibble is treated as the low nibble
                //   of a byte (e.g. `0xabc` => ... 0x0a 0xbc).
                fn hex_nibble_value(b: u8) -> Option<u8> {
                    match b {
                        b'0'..=b'9' => Some(b - b'0'),
                        b'a'..=b'f' => Some(b - b'a' + 10),
                        b'A'..=b'F' => Some(b - b'A' + 10),
                        _ => None,
                    }
                }

                let hex_bytes = hex.as_bytes();
                let mut out = [0u8; 32];
                let mut out_index = out.len();
                let mut hex_pos = hex_bytes.len();

                // Read from the end (least significant digits) and fill the output from the end.
                while hex_pos > 0 {
                    if out_index == 0 {
                        return Err(E::custom("hex string too long for 32 bytes"));
                    }

                    // Always read the low nibble.
                    let low_nibble =
                        hex_nibble_value(hex_bytes[hex_pos - 1]).ok_or_else(|| E::custom("invalid hex digit"))?;
                    hex_pos -= 1;

                    // Read the high nibble if present (otherwise treat as 0 for odd-length inputs).
                    let high_nibble = if hex_pos > 0 {
                        let v =
                            hex_nibble_value(hex_bytes[hex_pos - 1]).ok_or_else(|| E::custom("invalid hex digit"))?;
                        hex_pos -= 1;
                        v
                    } else {
                        0
                    };

                    out_index -= 1;
                    out[out_index] = (high_nibble << 4) | low_nibble;
                }

                Ok(L1TxnHash(out))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

/// Parameters for `starknet_getMessagesStatus` (v0.9.0+).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct GetMessagesStatusParams {
    /// The hash of the L1 transaction that sent L1->L2 messages.
    pub transaction_hash: L1TxnHash,
}

/// An item in the result array for `starknet_getMessagesStatus`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MessageStatus {
    pub transaction_hash: TxnHash,
    pub finality_status: TxnFinalityStatus,
    pub execution_status: TxnExecutionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[cfg(test)]
mod get_messages_status_tests {
    use super::*;

    #[test]
    fn l1_txn_hash_accepts_short_hex_and_left_pads() {
        let h: L1TxnHash = serde_json::from_str("\"0x1\"").unwrap();
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(h, L1TxnHash(expected));
        assert_eq!(
            serde_json::to_string(&h).unwrap(),
            "\"0x0000000000000000000000000000000000000000000000000000000000000001\""
        );
    }

    #[test]
    fn l1_txn_hash_accepts_odd_nibbles() {
        let h: L1TxnHash = serde_json::from_str("\"0xabc\"").unwrap();
        let mut expected = [0u8; 32];
        expected[30] = 0x0a;
        expected[31] = 0xbc;
        assert_eq!(h, L1TxnHash(expected));
    }

    #[test]
    fn l1_txn_hash_rejects_missing_prefix() {
        let err = serde_json::from_str::<L1TxnHash>("\"123\"").unwrap_err();
        assert!(err.to_string().contains("0x-prefixed"));
    }

    #[test]
    fn l1_txn_hash_rejects_too_long() {
        // 65 hex digits after 0x
        let s = format!("\"0x{}\"", "1".repeat(65));
        let err = serde_json::from_str::<L1TxnHash>(&s).unwrap_err();
        assert!(err.to_string().contains("at most 64"));
    }
}
