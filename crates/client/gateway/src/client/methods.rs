use mp_block::{BlockId, BlockTag};
use serde::{Deserialize, Serialize};

use crate::error::SequencerError;

use super::{builder::FeederClient, request_builder::RequestBuilder};

use mp_gateway::{
    block::{BlockProvider, PendingBlockProvider, ProviderMaybePendingBlock},
    state_update::{PendingStateUpdateProvider, ProviderMaybePendingStateUpdate, StateUpdateProvider},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateUpdateWithBlock {
    pub state_update: StateUpdateProvider,
    pub block: BlockProvider,
}

impl FeederClient {
    pub async fn get_block(&self, block_id: BlockId) -> Result<ProviderMaybePendingBlock, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_block")
            .unwrap()
            .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderMaybePendingBlock::Pending(request.send_get::<PendingBlockProvider>().await?))
            }
            _ => Ok(ProviderMaybePendingBlock::Block(request.send_get::<BlockProvider>().await?)),
        }
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<ProviderMaybePendingStateUpdate, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_state_update")
            .unwrap()
            .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderMaybePendingStateUpdate::Pending(request.send_get::<PendingStateUpdateProvider>().await?))
            }
            _ => Ok(ProviderMaybePendingStateUpdate::Update(request.send_get::<StateUpdateProvider>().await?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use mp_block::BlockTag;
    use starknet_core::types::Felt;

    use super::*;

    #[tokio::test]
    async fn test_get_block() {
        let client = FeederClient::starknet_alpha_mainnet();

        let block = client.get_block(BlockId::Number(0)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        let block = client
            .get_block(BlockId::Hash(Felt::from_hex_unchecked(
                "0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943",
            )))
            .await
            .unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        let block = client.get_block(BlockId::Tag(BlockTag::Latest)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        let block = client.get_block(BlockId::Tag(BlockTag::Pending)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
    }

    #[tokio::test]
    async fn test_get_state_update() {
        let client = FeederClient::starknet_alpha_mainnet();

        let block = client.get_state_update(BlockId::Number(0)).await.unwrap();
        let block = client
            .get_state_update(BlockId::Hash(Felt::from_hex_unchecked(
                "0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943",
            )))
            .await
            .unwrap();
        let block = client.get_state_update(BlockId::Tag(BlockTag::Latest)).await.unwrap();
        let block = client.get_state_update(BlockId::Tag(BlockTag::Pending)).await.unwrap();
    }
}
