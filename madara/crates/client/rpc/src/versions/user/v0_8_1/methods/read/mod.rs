use crate::versions::user::v0_8_1::StarknetReadRpcApiV0_8_1Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use mp_rpc::v0_8_1::{
    BlockId, BroadcastedTxn, ContractStorageKeysItem, FeeEstimate, GetStorageProofResult,
    MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs, MsgFromL1, SimulationFlagForEstimateFee,
    StarknetGetBlockWithTxsAndReceiptsResult,
};
use starknet_types_core::felt::Felt;

pub mod estimate_fee;
pub mod estimate_message_fee;
pub mod get_block_with_receipts;
pub mod get_block_with_tx_hashes;
pub mod get_block_with_txs;
pub mod get_compiled_casm;
pub mod get_storage_proof;

#[async_trait]
impl StarknetReadRpcApiV0_8_1Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_8_1.to_string())
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        Ok(get_compiled_casm::get_compiled_casm(self, class_hash)?)
    }

    fn get_storage_proof(
        &self,
        block_id: BlockId,
        class_hashes: Option<Vec<Felt>>,
        contract_addresses: Option<Vec<Felt>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeysItem>>,
    ) -> RpcResult<GetStorageProofResult> {
        get_storage_proof::get_storage_proof(self, block_id, class_hashes, contract_addresses, contracts_storage_keys)
    }

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlagForEstimateFee>,
        block_id: BlockId,
    ) -> RpcResult<Vec<FeeEstimate>> {
        Ok(estimate_fee::estimate_fee(self, request, simulation_flags, block_id).await?)
    }

    async fn estimate_message_fee(&self, message: MsgFromL1, block_id: BlockId) -> RpcResult<FeeEstimate> {
        Ok(estimate_message_fee::estimate_message_fee(self, message, block_id).await?)
    }

    fn get_block_with_receipts(&self, block_id: BlockId) -> RpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
        Ok(get_block_with_receipts::get_block_with_receipts(self, block_id)?)
    }

    fn get_block_with_tx_hashes(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
        Ok(get_block_with_tx_hashes::get_block_with_tx_hashes(self, block_id)?)
    }

    fn get_block_with_txs(&self, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
        Ok(get_block_with_txs::get_block_with_txs(self, block_id)?)
    }
}
