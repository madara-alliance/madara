//! v0.9.0 of the API.
pub use crate::custom::{BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn, SyncingStatus};

mod starknet_api_openrpc;
mod starknet_trace_api_openrpc;
mod starknet_ws_api;

pub use self::starknet_api_openrpc::*;
pub use self::starknet_trace_api_openrpc::*;
pub use self::starknet_ws_api::*;

pub use crate::v0_8_1::{
    AddDeclareTransactionParams, AddDeployAccountTransactionParams, AddInvokeTransactionParams,
    AddInvokeTransactionResult, Address, BlockHash, BlockHashAndNumber, BlockHashAndNumberParams, BlockHashHelper,
    BlockHeader, BlockNumber, BlockNumberHelper, BlockNumberParams, BroadcastedDeclareTxnV1, BroadcastedDeclareTxnV2,
    BroadcastedDeclareTxnV3, BroadcastedTxn, CallParams, CallType, ChainId, ChainIdParams, ClassAndTxnHash,
    ComputationResources, ContractAbi, ContractAbiEntry, ContractAndTxnHash, ContractClass, ContractLeavesDataItem,
    ContractStorageDiffItem, ContractStorageKeysItem, ContractsProof, DaMode, DataAvailability,
    DeclareTransactionTrace, DeclareTxn, DeclareTxnV0, DeclareTxnV1, DeclareTxnV2, DeclareTxnV3,
    DeployAccountTransactionTrace, DeployAccountTxn, DeployAccountTxnV1, DeployAccountTxnV3, DeployTxn,
    DeployedContractItem, DeprecatedCairoEntryPoint, DeprecatedContractClass, DeprecatedEntryPointsByType,
    EmittedEvent, EntryPointType, EntryPointsByType, EstimateFeeParams, EstimateMessageFeeParams, EthAddress, Event,
    EventAbiEntry, EventAbiType, EventContent, EventsChunk, ExecuteInvocation, ExecutionResources, ExecutionStatus,
    FunctionAbiEntry, FunctionAbiType, FunctionCall, FunctionInvocation, FunctionStateMutability,
    GetBlockTransactionCountParams, GetBlockWithReceiptsParams, GetBlockWithTxHashesParams, GetBlockWithTxsParams,
    GetClassAtParams, GetClassHashAtParams, GetClassParams, GetEventsParams, GetNonceParams, GetStateUpdateParams,
    GetStorageAtParams, GetStorageProofResult, GetTransactionByBlockIdAndIndexParams, GetTransactionByHashParams,
    GetTransactionReceiptParams, GetTransactionStatusParams, GlobalRoots, InvokeTransactionTrace, InvokeTxn,
    InvokeTxnV0, InvokeTxnV1, InvokeTxnV3, KeyValuePair, L1DaMode, L1HandlerTransactionTrace, L1HandlerTxn,
    MaybeDeprecatedContractClass, MerkleNode, MsgFromL1, MsgToL1, NestedCall, NewClasses, NodeHashToNodeMappingItem,
    NonceUpdate, OrderedEvent, OrderedMessage, ReplacedClass, ResourceBounds, ResourceBoundsMapping, ResourcePrice,
    RevertedInvocation, SierraEntryPoint, Signature, SimulateTransactionsParams, SimulationFlag,
    SimulationFlagForEstimateFee, SpecVersionParams, StateDiff, StateUpdate, StorageKey, StructAbiEntry, StructAbiType,
    StructMember, SyncStatus, SyncingParams, TraceBlockTransactionsParams, TraceBlockTransactionsResult,
    TraceTransactionParams, TraceTransactionResult, TransactionTrace, Txn, TxnHash, TxnWithHash, TypedParameter,
};

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
