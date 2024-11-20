use crate::versions::v0_8_0::methods::read::get_messages_status::{get_messages_status, MessageStatus};
use crate::versions::v0_8_0::StarknetReadRpcApiV0_8_0Server;
use crate::Starknet;
use alloy::primitives::TxHash;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_chain_config::RpcVersion;
use starknet_types_core::felt::Felt;

use super::get_compiled_casm::*;

use crate::versions::user::v0_8_0::StarknetReadRpcApiV0_8_0Server;
use crate::Starknet;

#[async_trait]
impl StarknetReadRpcApiV0_8_0Server for Starknet {
    fn spec_version(&self) -> RpcResult<String> {
        Ok(RpcVersion::RPC_VERSION_0_8_0.to_string())
    }

    fn get_compiled_casm(&self, class_hash: Felt) -> RpcResult<serde_json::Value> {
        Ok(get_compiled_casm(self, class_hash)?)
    }

    fn get_messages_status(&self, transaction_hash: TxHash) -> RpcResult<Vec<MessageStatus>> {
        Ok(get_messages_status(self, transaction_hash)?)
    }
}
