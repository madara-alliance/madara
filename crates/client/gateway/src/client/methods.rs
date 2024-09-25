use std::borrow::Cow;

use mp_block::{BlockId, BlockTag};
use mp_class::ContractClass;
use starknet_core::types::Felt;

use crate::error::SequencerError;

use super::{builder::FeederClient, request_builder::RequestBuilder};

use mp_gateway::{
    block::{ProviderBlock, ProviderBlockPending, ProviderBlockPendingMaybe},
    state_update::{
        ProviderStateUpdate, ProviderStateUpdatePending, ProviderStateUpdatePendingMaybe, ProviderStateUpdateWithBlock,
        ProviderStateUpdateWithBlockPending, ProviderStateUpdateWithBlockPendingMaybe,
    },
};

impl FeederClient {
    pub async fn get_block(&self, block_id: BlockId) -> Result<ProviderBlockPendingMaybe, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_block")
            .unwrap()
            .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderBlockPendingMaybe::Pending(request.send_get::<ProviderBlockPending>().await?))
            }
            _ => Ok(ProviderBlockPendingMaybe::Block(request.send_get::<ProviderBlock>().await?)),
        }
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<ProviderStateUpdatePendingMaybe, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_state_update")
            .unwrap()
            .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderStateUpdatePendingMaybe::Pending(request.send_get::<ProviderStateUpdatePending>().await?))
            }
            _ => Ok(ProviderStateUpdatePendingMaybe::Update(request.send_get::<ProviderStateUpdate>().await?)),
        }
    }

    pub async fn get_state_update_with_block(
        &self,
        block_id: BlockId,
    ) -> Result<ProviderStateUpdateWithBlockPendingMaybe, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_state_update")
            .unwrap()
            .with_block_id(block_id)
            .add_param(Cow::from("includeBlock"), "true");

        match block_id {
            BlockId::Tag(BlockTag::Pending) => Ok(ProviderStateUpdateWithBlockPendingMaybe::Pending(
                request.send_get::<ProviderStateUpdateWithBlockPending>().await?,
            )),
            _ => Ok(ProviderStateUpdateWithBlockPendingMaybe::UpdateWithBlock(
                request.send_get::<ProviderStateUpdateWithBlock>().await?,
            )),
        }
    }

    pub async fn get_class_by_hash(
        &self,
        class_hash: Felt,
        block_id: BlockId,
    ) -> Result<ContractClass, SequencerError> {
        let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone())
            .add_uri_segment("get_class_by_hash")
            .unwrap()
            .with_block_id(block_id)
            .with_class_hash(class_hash);

        Ok(request.send_get::<ContractClass>().await?)
    }
}

#[cfg(test)]
mod tests {
    use mp_block::BlockTag;
    use rstest::*;
    use serde::de::DeserializeOwned;
    use starknet_core::types::Felt;
    use std::fs::File;
    use std::io::BufReader;
    use std::path::{Path, PathBuf};

    use super::*;

    #[fixture]
    fn client_mainnet_fixture() -> FeederClient {
        FeederClient::starknet_alpha_mainnet()
    }

    #[rstest]
    #[tokio::test]
    async fn get_block(client_mainnet_fixture: FeederClient) {
        let block = client_mainnet_fixture.get_block(BlockId::Number(0)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        assert!(matches!(block, ProviderBlockPendingMaybe::Block(_)));
        assert_eq!(block.block().unwrap().block_number, 0);
        assert_eq!(block.block().unwrap().parent_block_hash, Felt::from_hex_unchecked("0x0"));

        let block = client_mainnet_fixture
            .get_block(BlockId::Hash(Felt::from_hex_unchecked(
                "0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943",
            )))
            .await
            .unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        let block = client_mainnet_fixture.get_block(BlockId::Tag(BlockTag::Latest)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        let block = client_mainnet_fixture.get_block(BlockId::Tag(BlockTag::Pending)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
    }

    #[rstest]
    #[tokio::test]
    async fn get_state_update(client_mainnet_fixture: FeederClient) {
        let state_update = client_mainnet_fixture.get_state_update(BlockId::Number(0)).await.unwrap();
        assert!(matches!(state_update, ProviderStateUpdatePendingMaybe::Update(_)));
        assert_eq!(
            state_update.state_update().unwrap().block_hash,
            Felt::from_hex_unchecked("0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943")
        );

        let _block = client_mainnet_fixture
            .get_state_update(BlockId::Hash(Felt::from_hex_unchecked(
                "0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943",
            )))
            .await
            .unwrap();
        let _block = client_mainnet_fixture.get_state_update(BlockId::Tag(BlockTag::Latest)).await.unwrap();
        let _block = client_mainnet_fixture.get_state_update(BlockId::Tag(BlockTag::Pending)).await.unwrap();
    }

    #[rstest]
    #[tokio::test]
    async fn get_state_update_with_block(client_mainnet_fixture: FeederClient) {
        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(0))
            .await
            .expect("Getting state update and block at block number 0");
        let state_update_with_block_0 = let_binding
            .state_update_with_block()
            .expect("State update with block at block number 0 should not be pending");
        let state_update_with_block_0_reference =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_0.json");

        assert_eq!(state_update_with_block_0, &state_update_with_block_0_reference);

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(1))
            .await
            .expect("Getting state update and block at block number 1");
        let state_update_with_block_1 = let_binding
            .state_update_with_block()
            .expect("State update with block at block number 1 should not be pending");
        let state_update_with_block_1_reference =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_1.json");

        assert_eq!(state_update_with_block_1, &state_update_with_block_1_reference);

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(2))
            .await
            .expect("Getting state update and block at block number 2");
        let state_update_with_block_2 = let_binding
            .state_update_with_block()
            .expect("State update with block at block number 2 should not be pending");
        let state_update_with_block_2_reference =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_2.json");

        assert_eq!(state_update_with_block_2, &state_update_with_block_2_reference);

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Tag(BlockTag::Latest))
            .await
            .expect("Getting state update and block at block latest");
        let state_update_with_block_latest = let_binding
            .state_update_with_block()
            .expect("State update with block at block latest should not be pending");

        assert!(matches!(state_update_with_block_latest, ProviderStateUpdateWithBlock));

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Tag(BlockTag::Pending))
            .await
            .expect("Getting state update and block at block latest");
        let state_update_with_block_pending =
            let_binding.pending().expect("State update with block at block latest should be pending");

        assert!(matches!(state_update_with_block_pending, ProviderStateUpdateWithBlockPending));
    }

    fn load_from_file<T>(path: &str) -> T
    where
        T: DeserializeOwned,
    {
        let mut path_abs = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path_abs.push(path);
        let file = File::open(&path_abs).expect(&format!("loading test mock from {path_abs:?}"));
        let reader = BufReader::new(file);

        serde_json::from_reader(reader).expect(&format!("deserializing test mock from {path_abs:?}"))
    }
}
