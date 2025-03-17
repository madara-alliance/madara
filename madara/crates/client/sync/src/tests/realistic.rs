use super::gateway_mock::{gateway_mock, GatewayMock};
use crate::{
    gateway::ForwardSyncConfig,
    import::{BlockImporter, BlockValidationConfig},
    SyncControllerConfig,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_chain_config::ChainConfig;
use mp_receipt::{Event, EventWithTransactionHash};
use mp_state_update::NonceUpdate;
use mp_utils::service::ServiceContext;
use rstest::{fixture, rstest};
use starknet_api::felt;
use std::sync::Arc;
use tracing_test::traced_test;

struct TestContext {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    gateway_mock: GatewayMock,
}

impl TestContext {
    async fn sync_to(&self, block_n: u64) {
        let mut sync = crate::gateway::forward_sync(
            self.backend.clone(),
            self.importer.clone(),
            self.gateway_mock.client(),
            SyncControllerConfig::default().stop_on_sync(true).stop_at_block_n(Some(block_n)),
            ForwardSyncConfig::default(),
        );

        sync.run(ServiceContext::new_for_testing()).await.unwrap();
    }
}

#[fixture]
fn ctx(gateway_mock: GatewayMock) -> TestContext {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::starknet_sepolia()));
    let importer = Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default()));

    gateway_mock.mock_block_from_json(0, include_str!("../../resources/sepolia.block_0.json"));
    gateway_mock.mock_class_from_json(
        "0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3",
        include_str!("../../resources/sepolia.block_0_class_0.json"),
    );
    gateway_mock.mock_class_from_json(
        "0x5c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6",
        include_str!("../../resources/sepolia.block_0_class_1.json"),
    );
    gateway_mock.mock_block_from_json(1, include_str!("../../resources/sepolia.block_1.json"));
    gateway_mock.mock_class_from_json(
        "0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1",
        include_str!("../../resources/sepolia.block_1_class_0.json"),
    );
    gateway_mock.mock_block_from_json(2, include_str!("../../resources/sepolia.block_2.json"));
    gateway_mock.mock_class_from_json(
        "0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1",
        include_str!("../../resources/sepolia.block_2_class_0.json"),
    );
    gateway_mock.mock_block_pending_not_found();
    gateway_mock.mock_header_latest(2, felt!("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d"));

    TestContext { backend, importer, gateway_mock }
}

#[rstest]
#[tokio::test]
#[traced_test]
/// This test makes sure that the pipeline actually imports stuff, so that we're sure that we
/// didn't forget to call the store functions in the backend.
///
/// We do not test thoroughly all of the fields, this kind of test would be the responsability of the gateway client;
/// because parsing is implemented there.
///
/// We check that import is successful:
/// - [x] block hashes match (header)
/// - [x] classes are imported
/// - [x] check some transactions
/// - [x] check some events
/// - [x] check parts of the state diff
async fn test_should_import(ctx: TestContext) {
    // block 0
    ctx.sync_to(0).await;
    let id = DbBlockId::Number(0);
    assert_eq!(
        ctx.backend.get_block_hash(&id).unwrap().unwrap(),
        felt!("0x5c627d4aeb51280058bed93c7889bce78114d63baad1be0f0aeb32496d5f19c")
    );
    // Classes
    assert!(ctx
        .backend
        .get_class_info(&id, &felt!("0xd0e183745e9dae3e4e78a8ffedcce0903fc4900beace4e0abf192d4c202da3"))
        .unwrap()
        .is_some());
    assert!(ctx
        .backend
        .get_class_info(&id, &felt!("0x5c478ee27f2112411f86f207605b2e2c58cdb647bac0df27f660ef2252359c6"))
        .unwrap()
        .is_some());
    // State diff
    assert_eq!(
        ctx.backend.get_block_state_diff(&id).unwrap().unwrap().nonces,
        vec![NonceUpdate {
            contract_address: felt!("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8"),
            nonce: felt!("0x5")
        }]
    );
    assert_eq!(
        ctx.backend
            .get_contract_storage_at(
                &id,
                &felt!("0x4c5772d1914fe6ce891b64eb35bf3522aeae1315647314aac58b01137607f3f"),
                &felt!("0xe8fc4f1b6b3dc661208f9a8a5017a6c059098327e31518722e0a5c3a5a7e86")
            )
            .unwrap()
            .unwrap(),
        felt!("0x1")
    );
    assert_eq!(
        ctx.backend
            .get_contract_storage_at(
                &id,
                &felt!("0x49d36570d4e46f48e9dddddddddddddddd"),
                &felt!("0xe8fc4f1b6b3dc661208f9a8a5017a6c059098327e31518722e0a5c3a5a7e86")
            )
            .unwrap(),
        None
    );
    // Transactions
    let inner = ctx.backend.get_block_inner(&id).unwrap().unwrap();
    assert_eq!(inner.transactions.len(), 7);
    assert_eq!(inner.receipts.len(), 7);
    assert_eq!(
        inner.receipts[3].transaction_hash(),
        felt!("0x6a5a493cf33919e58aa4c75777bffdef97c0e39cac968896d7bee8cc67905a1")
    );
    assert_eq!(
        inner.transactions[5].as_invoke().unwrap().signature(),
        &[
            felt!("0x44580ba3cd68e5d9509d2fcb8bd09933ae4a7b7dfe6744eaa2329f9a79d7408"),
            felt!("0x68404a4da22c31d8367f873c043318750a24c669702c72de8518a3d52284b94")
        ]
    );
    // Events
    let events = inner.events().collect::<Vec<_>>();
    assert_eq!(events.len(), 4);
    assert_eq!(
        events[1],
        EventWithTransactionHash {
            transaction_hash: felt!("0x1bec64a9f5ff52154b560fd489ae2aabbfcb31062f7ea70c3c674ddf14b0add"),
            event: Event {
                from_address: felt!("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
                keys: vec![felt!("0x4595132f9b33b7077ebf2e7f3eb746a8e0a6d5c337c71cd8f9bf46cac3cfd7")],
                data: vec![felt!("0x43abaa073c768ebf039c0c4f46db9acc39e9ec165690418060a652aab39e7d8")]
            }
        }
    );

    // block 1
    ctx.sync_to(1).await;
    let id = DbBlockId::Number(1);
    assert_eq!(
        ctx.backend.get_block_hash(&id).unwrap().unwrap(),
        felt!("0x78b67b11f8c23850041e11fb0f3b39db0bcb2c99d756d5a81321d1b483d79f6")
    );
    // Classes
    assert!(ctx
        .backend
        .get_class_info(&id, &felt!("0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1"))
        .unwrap()
        .is_some());
    // State diff
    assert_eq!(
        ctx.backend.get_block_state_diff(&id).unwrap().unwrap().deprecated_declared_classes,
        vec![felt!("0x1b661756bf7d16210fc611626e1af4569baa1781ffc964bd018f4585ae241c1")]
    );

    // block 2
    ctx.sync_to(2).await;
    let id = DbBlockId::Number(2);
    assert_eq!(
        ctx.backend.get_block_hash(&id).unwrap().unwrap(),
        felt!("0x7a906dfd1ff77a121b8048e6f750cda9e949d341c4487d4c6a449f183f0e61d")
    );
    assert!(ctx
        .backend
        .get_class_info(&id, &felt!("0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1"))
        .unwrap()
        .is_some());
    // State diff
    assert_eq!(
        ctx.backend.get_block_state_diff(&id).unwrap().unwrap().deprecated_declared_classes,
        vec![felt!("0x4f23a756b221f8ce46b72e6a6b10ee7ee6cf3b59790e76e02433104f9a8c5d1")]
    );
    // Transaction
    let inner = ctx.backend.get_block_inner(&id).unwrap().unwrap();
    assert_eq!(inner.transactions.len(), 1);
    assert_eq!(inner.receipts.len(), 1);
    assert_eq!(inner.receipts[0].execution_resources().steps, 2711);
}

#[fixture]
fn ctx_mainnet(gateway_mock: GatewayMock) -> TestContext {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::starknet_mainnet()));
    let importer = Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default()));

    gateway_mock.mock_block_from_json(0, include_str!("../../resources/mainnet.block_0.json"));
    gateway_mock.mock_class_from_json(
        "0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8",
        include_str!("../../resources/mainnet.block_0_class_0.json"),
    );
    gateway_mock.mock_block_pending_not_found();
    gateway_mock.mock_header_latest(0, felt!("0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943"));

    TestContext { backend, importer, gateway_mock }
}

#[rstest]
#[tokio::test]
#[traced_test]
/// Mainnet has some classes in the very early history that don't appear in the
/// state diff. This test ensures we import them, but it also test the general correctness
/// for the mainnet block 0.
async fn test_realistic_mainnet(ctx_mainnet: TestContext) {
    // block 0
    ctx_mainnet.sync_to(0).await;
    let id = DbBlockId::Number(0);
    assert_eq!(
        ctx_mainnet.backend.get_block_hash(&id).unwrap().unwrap(),
        felt!("0x47c3637b57c2b079b93c61539950c17e868a28f46cdef28f88521067f21e943")
    );
    // Classes
    assert!(ctx_mainnet
        .backend
        .get_class_info(&id, &felt!("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8"))
        .unwrap()
        .is_some()); // should find!
                     // State diff
    let state_diff = ctx_mainnet.backend.get_block_state_diff(&id).unwrap().unwrap();
    assert_eq!(state_diff.deprecated_declared_classes, vec![]); // empty
    assert_eq!(state_diff.deployed_contracts.len(), 5);
    assert_eq!(
        state_diff.deployed_contracts[3].address,
        felt!("0x6ee3440b08a9c805305449ec7f7003f27e9f7e287b83610952ec36bdc5a6bae")
    );
    assert_eq!(
        state_diff.deployed_contracts[3].class_hash,
        felt!("0x10455c752b86932ce552f2b0fe81a880746649b9aee7e0d842bf3f52378f9f8")
    );
    assert_eq!(
        ctx_mainnet
            .backend
            .get_contract_storage_at(
                &id,
                &felt!("0x31c9cdb9b00cb35cf31c05855c0ec3ecf6f7952a1ce6e3c53c3455fcd75a280"),
                &felt!("0x5fac6815fddf6af1ca5e592359862ede14f171e1544fd9e792288164097c35d")
            )
            .unwrap()
            .unwrap(),
        felt!("0x299e2f4b5a873e95e65eb03d31e532ea2cde43b498b50cd3161145db5542a5")
    );
}
