use serde::{Deserialize, Deserializer, Serialize};

use crate::v0_7_1::{BlockHash, BlockNumber, BlockTag};

/// A hexadecimal number.
#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum BlockId {
    /// The tag of the block.
    Tag(BlockTag),
    /// The hash of the block.
    Hash(BlockHash),
    /// The height of the block.
    Number(BlockNumber),
}

#[derive(Serialize, Deserialize)]
struct BlockHashHelper {
    block_hash: BlockHash,
}

#[derive(Serialize, Deserialize)]
struct BlockNumberHelper {
    block_number: BlockNumber,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BlockIdHelper {
    Tag(BlockTag),
    Hash(BlockHashHelper),
    Number(BlockNumberHelper),
}

impl serde::Serialize for BlockId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            BlockId::Tag(tag) => tag.serialize(serializer),
            BlockId::Hash(block_hash) => {
                let helper = BlockHashHelper { block_hash: *block_hash };
                helper.serialize(serializer)
            }
            BlockId::Number(block_number) => {
                let helper = BlockNumberHelper { block_number: *block_number };
                helper.serialize(serializer)
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for BlockId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let helper = BlockIdHelper::deserialize(deserializer)?;
        match helper {
            BlockIdHelper::Tag(tag) => Ok(BlockId::Tag(tag)),
            BlockIdHelper::Hash(helper) => Ok(BlockId::Hash(helper.block_hash)),
            BlockIdHelper::Number(helper) => Ok(BlockId::Number(helper.block_number)),
        }
    }
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
fn block_id_from_pending() {
    let s = "\"pending\"";
    let block_id: BlockId = serde_json::from_str(s).unwrap();
    assert_eq!(block_id, BlockId::Tag(BlockTag::Pending));
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
fn block_id_to_pending() {
    let block_id = BlockId::Tag(BlockTag::Pending);
    let s = serde_json::to_string(&block_id).unwrap();
    assert_eq!(s, "\"pending\"");
}
