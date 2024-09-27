use std::{borrow::Cow, sync::Arc};

use mp_block::{BlockId, BlockTag};
use mp_class::{CompressedLegacyContractClass, ContractClass, FlattenedSierraClass};
use starknet_core::types::{contract::legacy::LegacyContractClass, Felt};

use crate::error::{SequencerError, StarknetError};

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
        let request =
            RequestBuilder::new_with_headers(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_block")
                .unwrap()
                .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderBlockPendingMaybe::Pending(request.send_get::<ProviderBlockPending>().await?))
            }
            _ => Ok(ProviderBlockPendingMaybe::NonPending(request.send_get::<ProviderBlock>().await?)),
        }
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<ProviderStateUpdatePendingMaybe, SequencerError> {
        let request =
            RequestBuilder::new_with_headers(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_state_update")
                .unwrap()
                .with_block_id(block_id);

        match block_id {
            BlockId::Tag(BlockTag::Pending) => {
                Ok(ProviderStateUpdatePendingMaybe::Pending(request.send_get::<ProviderStateUpdatePending>().await?))
            }
            _ => Ok(ProviderStateUpdatePendingMaybe::NonPending(request.send_get::<ProviderStateUpdate>().await?)),
        }
    }

    pub async fn get_state_update_with_block(
        &self,
        block_id: BlockId,
    ) -> Result<ProviderStateUpdateWithBlockPendingMaybe, SequencerError> {
        let request =
            RequestBuilder::new_with_headers(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_state_update")
                .unwrap()
                .with_block_id(block_id)
                .add_param(Cow::from("includeBlock"), "true");

        match block_id {
            BlockId::Tag(BlockTag::Pending) => Ok(ProviderStateUpdateWithBlockPendingMaybe::Pending(
                request.send_get::<ProviderStateUpdateWithBlockPending>().await?,
            )),
            _ => Ok(ProviderStateUpdateWithBlockPendingMaybe::NonPending(
                request.send_get::<ProviderStateUpdateWithBlock>().await?,
            )),
        }
    }

    pub async fn get_class_by_hash(
        &self,
        class_hash: Felt,
        block_id: BlockId,
    ) -> Result<ContractClass, SequencerError> {
        let request =
            RequestBuilder::new_with_headers(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_class_by_hash")
                .unwrap()
                .with_block_id(block_id)
                .with_class_hash(class_hash);

        let response = request.send_get_raw().await?;
        let status = response.status();
        if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR || status == reqwest::StatusCode::BAD_REQUEST {
            let error = match response.json::<StarknetError>().await {
                Ok(e) => SequencerError::StarknetError(e),
                Err(e) if e.is_decode() => SequencerError::InvalidStarknetErrorVariant(e),
                Err(e) => SequencerError::ReqwestError(e),
            };
            return Err(error);
        }

        let bytes = response.bytes().await?;
        match serde_json::from_slice::<FlattenedSierraClass>(&bytes) {
            Ok(class_sierra) => Ok(ContractClass::Sierra(Arc::new(class_sierra))),
            Err(_) => {
                let class_legacy = serde_json::from_slice::<LegacyContractClass>(&bytes)?;
                let class_compressed: CompressedLegacyContractClass = class_legacy.compress()?.into();
                Ok(ContractClass::Legacy(Arc::new(class_compressed)))
            }
        }
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
    use std::path::PathBuf;

    use super::*;

    const CLASS_BLOCK_0: &str = "0x010455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8";

    const CLASS_ACCOUNT: &str = "0x07595b4f7d50010ceb00230d8b5656e3c3dd201b6df35d805d3f2988c69a1432";
    const CLASS_ACCOUNT_BLOCK: u64 = 1342;

    const CLASS_PROXY: &str = "0x071c3c99f5cf76fc19945d4b8b7d34c7c5528f22730d56192b50c6bbfd338a64";
    const CLASS_PROXY_BLOCK: u64 = 1343;

    const CLASS_ERC20: &str = "0x07543f8eb21f10b1827a495084697a519274ac9c1a1fbf931bac40133a6b9c15";
    const CLASS_ERC20_BLOCK: u64 = 1981;

    const CLASS_ERC721: &str = "0x074a7ed7f1236225600f355efe70812129658c82c295ff0f8307b3fad4bf09a9";
    const CLASS_ERC721_BLOCK: u64 = 3125;

    const CLASS_ERC1155: &str = "0x04be7f1bace6f593abd8e56947c11151f45498030748a950fdaf0b79ac3dc03f";
    const CLASS_ERC1155_BLOCK: u64 = 18507;

    /// Loads a json file, deserializing it into the target type.
    ///
    /// This should NOT be used for mocking behavior, but is fine for loading
    /// golden files to validate the output of a function, as long as this
    /// function is deterministic.
    ///
    /// * `path`: path to the file to deserialize
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

    #[fixture]
    fn client_mainnet_fixture() -> FeederClient {
        FeederClient::starknet_alpha_mainnet()
    }

    #[rstest]
    #[tokio::test]
    async fn get_block(client_mainnet_fixture: FeederClient) {
        let block = client_mainnet_fixture.get_block(BlockId::Number(0)).await.unwrap();
        println!("parent_block_hash: 0x{:x}", block.parent_block_hash());
        assert!(matches!(block, ProviderBlockPendingMaybe::NonPending(_)));
        assert_eq!(block.non_pending().unwrap().block_number, 0);
        assert_eq!(block.non_pending().unwrap().parent_block_hash, Felt::from_hex_unchecked("0x0"));

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
        assert!(matches!(state_update, ProviderStateUpdatePendingMaybe::NonPending(_)));
        assert_eq!(
            state_update.non_pending().unwrap().block_hash,
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

    // INFO:
    // These next few tests work by loading a golden file representing the
    // expected output of the feeder gateway. This is then manually
    // deserialized and compared to the result obtained by the fgw provider.
    //
    // Why not write down the expected results in Rust directly? Because this
    // would be error-prone and a pain to manage if the structures were updated.
    // Ideally we would want to perform this as an integration test against a
    // local devnet where we have total control over the testing environment,
    // removing the need for golden files. However, this is not yet possible.

    #[rstest]
    #[tokio::test]
    async fn get_state_update_with_block_first_few_blocks(client_mainnet_fixture: FeederClient) {
        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(0))
            .await
            .expect("Getting state update and block at block number 0");
        let (state_update_0, block_0) = let_binding.as_update_and_block();
        let state_update_0 =
            state_update_0.non_pending_ownded().expect("State update at block number 0 should not be pending");
        let block_0 = block_0.non_pending_owned().expect("Block at block number 0 should not be pending");
        let ProviderStateUpdateWithBlock { state_update: state_update_0_reference, block: block_0_reference } =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_0.json");

        assert_eq!(state_update_0, state_update_0_reference);
        assert_eq!(block_0, block_0_reference);

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(1))
            .await
            .expect("Getting state update and block at block number 1");
        let (state_update_1, block_1) = let_binding.as_update_and_block();
        let state_update_1 =
            state_update_1.non_pending_ownded().expect("State update at block number 1 should not be pending");
        let block_1 = block_1.non_pending_owned().expect("Block at block number 1 should not be pending");
        let ProviderStateUpdateWithBlock { state_update: state_update_1_reference, block: block_1_reference } =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_1.json");

        assert_eq!(state_update_1, state_update_1_reference);
        assert_eq!(block_1, block_1_reference);

        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Number(2))
            .await
            .expect("Getting state update and block at block number 2");
        let (state_update_2, block_2) = let_binding.as_update_and_block();
        let state_update_2 =
            state_update_2.non_pending_ownded().expect("State update at block number 0 should not be pending");
        let block_2 = block_2.non_pending_owned().expect("Block at block number 0 should not be pending");
        let ProviderStateUpdateWithBlock { state_update: state_update_2_reference, block: block_2_reference } =
            load_from_file::<ProviderStateUpdateWithBlock>("src/client/mocks/state_update_and_block_2.json");

        assert_eq!(state_update_2, state_update_2_reference);
        assert_eq!(block_2, block_2_reference);
    }

    #[rstest]
    #[tokio::test]
    async fn get_state_update_with_block_latest(client_mainnet_fixture: FeederClient) {
        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Tag(BlockTag::Latest))
            .await
            .expect("Getting state update and block at block latest");
        let (state_update_latest, block_latest) = let_binding.as_update_and_block();

        assert!(matches!(state_update_latest, ProviderStateUpdatePendingMaybe::NonPending(_)));
        assert!(matches!(block_latest, ProviderBlockPendingMaybe::NonPending(_)));
    }

    #[rstest]
    #[tokio::test]
    async fn get_state_update_with_block_pending(client_mainnet_fixture: FeederClient) {
        let let_binding = client_mainnet_fixture
            .get_state_update_with_block(BlockId::Tag(BlockTag::Pending))
            .await
            .expect("Getting state update and block at block latest");
        let (state_update_pending, block_pending) = let_binding.as_update_and_block();

        assert!(matches!(state_update_pending, ProviderStateUpdatePendingMaybe::Pending(_)));
        assert!(matches!(block_pending, ProviderBlockPendingMaybe::Pending(_)))
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_block_0(client_mainnet_fixture: FeederClient) {
        let class = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_BLOCK_0), BlockId::Number(0))
            .await
            .expect(&format!("Getting class {CLASS_BLOCK_0} at block number 0"));
        let class_reference =
            load_from_file::<LegacyContractClass>(&format!("src/client/mocks/class_block_0_{CLASS_BLOCK_0}.json"));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class, class_compressed_reference.into());
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_account(client_mainnet_fixture: FeederClient) {
        let class_account = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_ACCOUNT), BlockId::Number(CLASS_ACCOUNT_BLOCK))
            .await
            .expect(&format!("Getting account class {CLASS_ACCOUNT} at block number {CLASS_ACCOUNT_BLOCK}"));
        let class_reference = load_from_file::<LegacyContractClass>(&format!(
            "src/client/mocks/class_block_{CLASS_ACCOUNT_BLOCK}_account_{CLASS_ACCOUNT}.json"
        ));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class_account, class_compressed_reference.into());
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_proxy(client_mainnet_fixture: FeederClient) {
        let class_proxy = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_PROXY), BlockId::Number(CLASS_PROXY_BLOCK))
            .await
            .expect(&format!("Getting proxy class {CLASS_PROXY} at block number {CLASS_PROXY_BLOCK}"));
        let class_reference = load_from_file::<LegacyContractClass>(&format!(
            "src/client/mocks/class_block_{CLASS_PROXY_BLOCK}_proxy_{CLASS_PROXY}.json"
        ));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class_proxy, class_compressed_reference.into());
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_erc20(client_mainnet_fixture: FeederClient) {
        let class_erc20 = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_ERC20), BlockId::Number(CLASS_ERC20_BLOCK))
            .await
            .expect(&format!("Getting proxy class {CLASS_ERC20} at block number {CLASS_ERC20_BLOCK}"));
        let class_reference = load_from_file::<LegacyContractClass>(&format!(
            "src/client/mocks/class_block_{CLASS_ERC20_BLOCK}_erc20_{CLASS_ERC20}.json"
        ));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class_erc20, class_compressed_reference.into());
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_erc721(client_mainnet_fixture: FeederClient) {
        let class_erc721 = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_ERC721), BlockId::Number(CLASS_ERC721_BLOCK))
            .await
            .expect(&format!("Getting proxy class {CLASS_ERC721} at block number {CLASS_ERC721_BLOCK}"));
        let class_reference = load_from_file::<LegacyContractClass>(&format!(
            "src/client/mocks/class_block_{CLASS_ERC721_BLOCK}_erc721_{CLASS_ERC721}.json"
        ));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class_erc721, class_compressed_reference.into());
    }

    #[rstest]
    #[tokio::test]
    async fn get_class_by_hash_erc1155(client_mainnet_fixture: FeederClient) {
        let class_erc1155 = client_mainnet_fixture
            .get_class_by_hash(Felt::from_hex_unchecked(CLASS_ERC1155), BlockId::Number(CLASS_ERC1155_BLOCK))
            .await
            .expect(&format!("Getting proxy class {CLASS_ERC1155} at block number {CLASS_ERC1155_BLOCK}"));
        let class_reference = load_from_file::<LegacyContractClass>(&format!(
            "src/client/mocks/class_block_{CLASS_ERC1155_BLOCK}_erc1155_{CLASS_ERC1155}.json"
        ));
        let class_compressed_reference: CompressedLegacyContractClass =
            class_reference.compress().expect("Compressing legacy contract class").into();

        assert_eq!(class_erc1155, class_compressed_reference.into());
    }
}
