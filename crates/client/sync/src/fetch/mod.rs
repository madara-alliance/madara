use std::sync::Arc;
use std::time::Duration;

use futures::prelude::*;
use mc_block_import::UnverifiedFullBlock;
use mc_db::MadaraBackend;
use mp_utils::{channel_wait_or_graceful_shutdown, wait_or_graceful_shutdown};
use starknet_core::types::StarknetError;
use starknet_providers::{ProviderError, SequencerGatewayProvider};
use tokio::sync::{mpsc, oneshot};

use crate::fetch::fetchers::fetch_block_and_updates;

pub mod fetchers;

#[allow(clippy::too_many_arguments)]
pub async fn l2_fetch_task(
    backend: Arc<MadaraBackend>,
    first_block: u64,
    n_blocks_to_sync: Option<u64>,
    fetch_stream_sender: mpsc::Sender<UnverifiedFullBlock>,
    provider: Arc<SequencerGatewayProvider>,
    sync_polling_interval: Option<Duration>,
    once_caught_up_callback: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    // First, catch up with the chain
    let backend = &backend;

    let mut next_block = first_block;

    {
        // Fetch blocks and updates in parallel one time before looping
        let fetch_stream = (first_block..).take(n_blocks_to_sync.unwrap_or(u64::MAX) as _).map(|block_n| {
            let provider = Arc::clone(&provider);
            async move { (block_n, fetch_block_and_updates(backend, block_n, &provider).await) }
        });

        // Have 10 fetches in parallel at once, using futures Buffered
        let mut fetch_stream = stream::iter(fetch_stream).buffered(10);
        while let Some((block_n, val)) = channel_wait_or_graceful_shutdown(fetch_stream.next()).await {
            log::debug!("got {:?}", block_n);

            match val {
                Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                    log::info!("🥳 The sync process has caught up with the tip of the chain");
                    break;
                }
                val => {
                    if fetch_stream_sender.send(val?).await.is_err() {
                        // join error
                        break;
                    }
                }
            }

            next_block = block_n + 1;
        }
    };

    log::debug!("caught up with tip");
    let _ = once_caught_up_callback.send(());

    if let Some(sync_polling_interval) = sync_polling_interval {
        // Polling

        let mut interval = tokio::time::interval(sync_polling_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while wait_or_graceful_shutdown(interval.tick()).await.is_some() {
            loop {
                match fetch_block_and_updates(backend, next_block, &provider).await {
                    Err(FetchError::Provider(ProviderError::StarknetError(StarknetError::BlockNotFound))) => {
                        break;
                    }
                    val => {
                        if fetch_stream_sender.send(val?).await.is_err() {
                            // stream closed
                            break;
                        }
                    }
                }

                next_block += 1;
            }
        }
    }
    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum FetchError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod test_l2_fetch_task {
    use super::*;
    use dp_chain_config::ChainConfig;
    use httpmock::MockServer;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot};
    use url::Url;

    #[tokio::test]
    async fn test_basic_functionality() {
        let mock_server = MockServer::start();

        println!("Mock server URL: {}", mock_server.base_url());

        // Create SequencerGatewayProvider with mock server URL
        let provider = Arc::new(SequencerGatewayProvider::new(
            Url::parse(&format!("{}/gateway", mock_server.base_url())).unwrap(),
            Url::parse(&format!("{}/feeder_gateway", mock_server.base_url())).unwrap(),
            Felt::from_hex_unchecked("0x4d41444152415f54455354"), // Dummy chain ID
        ));

        // Initialize database service
        let chain_config = Arc::new(ChainConfig::test_config());
        let backend = DeoxysBackend::open_for_testing(chain_config.clone());

        // Create channels
        let (fetch_stream_sender, mut fetch_stream_receiver) = mpsc::channel(100);
        let (once_caught_up_sender, once_caught_up_receiver) = oneshot::channel();

        // Mock server to return the actual state update and block
        mock_server.mock(|when, then| {
            when.method("GET");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{
                    "block_hash": "0x5fd05f81f247bf6fc43dd7ba20cc05fbb20feccfa0fe5ec4739bdad6f46fb8c",
                    "new_root": "0x56792e6fd57ed164c38d68821a9065e21e6bd0254bb58dae0bd745fbff7bc1b",
                    "old_root": "0x561bd0f7f63a8044f0e497f301af8c9cbc8224b740bbd9c67cd2d0c7b3f2acb",
                    "state_diff": {
                        "storage_diffs": {
                            "0x601b0b001bb78d533f3febcb337a04a92159f84676a65691d77f1ab912ade38": [
                                {
                                    "key": "0x4db652a9af00e11cc04e7ca6bfc656beab4327e65b3fe01ab1b1139d73d2033",
                                    "value": "0x446bc71b024799e5e1add6d76038177b438affb051d5527cbb885702d82c36a"
                                }
                            ],
                            "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7": [
                                {
                                    "key": "0x1c9a3cb76acfabbba0abccd0a3bad33e6e2a5af5e42873d3af9e6ac3a0e3992",
                                    "value": "0x15f5e763c5331ac"
                                },
                                {
                                    "key": "0x5496768776e3db30053404f18067d81a6e06f5a2b0de326e21298fd9d569a9a",
                                    "value": "0x3570a9e85e58b"
                                }
                            ],
                            "0x1": [
                                {
                                    "key": "0x11",
                                    "value": "0x641077df21f3d3167e62ddf3feea236e2644e427940673114a30f2eb5c445c6"
                                }
                            ],
                            "0x56ee3c6cb7ad5262dc031a9a7d693feb77f56eae5341aacc0bf5cf37766f8d8": [
                                {
                                    "key": "0x2ba1e9106786fe90b2666d877ed7c15238ea399dccb14d490dec118d676a02e",
                                    "value": "0x7c87e32bae977db270b344ec731df28b8a91bf033c4c73c028215e513a190b0"
                                }
                            ],
                            "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037": [
                                {
                                    "key": "0x50cb77d5a306d42974091f893a905767056e8d10c667cecc3c581ec00c8770",
                                    "value": "0x12d7a52791baab55582f7d3ae7c9ca0b0629a4af8eedfb10da24b4a3c5fc0ff"
                                },
                                {
                                    "key": "0x50cb77d5a306d42974091f893a905767056e8d10c667cecc3c581ec00c8771",
                                    "value": "0x69c6162cbe927fca691b6ad3258e278e0c5c8bf732d7989fb78ada7f1ab10aa"
                                }
                            ]
                        },
                        "deployed_contracts": [
                            {
                                "address": "0x56ee3c6cb7ad5262dc031a9a7d693feb77f56eae5341aacc0bf5cf37766f8d8",
                                "class_hash": "0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0"
                            },
                            {
                                "address": "0x601b0b001bb78d533f3febcb337a04a92159f84676a65691d77f1ab912ade38",
                                "class_hash": "0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0"
                            }
                        ],
                        "old_declared_contracts": [],
                        "declared_classes": [],
                        "nonces": {
                            "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655": "0xc"
                        },
                        "replaced_classes": []
                    },
                    "block": {
                        "block_hash": "0x5fd05f81f247bf6fc43dd7ba20cc05fbb20feccfa0fe5ec4739bdad6f46fb8c",
                        "block_number": 27,
                        "parent_block_hash": "0x4f347ba366397064fe1280d77b23a03fb472f844efc037b7caee88b20fc541e",
                        "timestamp": 1701097328,
                        "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                        "state_root": "0x56792e6fd57ed164c38d68821a9065e21e6bd0254bb58dae0bd745fbff7bc1b",
                        "transaction_commitment": "0x7c4bb9259f9ce83200f9e657355bfd887a1bafa53fa026e3f10da6dd5373b39",
                        "event_commitment": "0x2912d2d366dbc6d0d41719038597a9c25114780859b535ce2fa746e075fac76",
                        "status": "ACCEPTED_ON_L1",
                        "l1_da_mode": "CALLDATA",
                        "l1_gas_price": {"price_in_fri": "0x0", "price_in_wei": "0x418c878a"},
                        "l1_data_gas_price": {"price_in_fri": "0x1", "price_in_wei": "0x1"},
                        "transactions": [
                            {
                                "type": "INVOKE_FUNCTION",
                                "sender_address": "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                "calldata": [
                                    "0x1",
                                    "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037",
                                    "0x7772be8b80a8a33dc6c1f9a6ab820c02e537c73e859de67f288c70f92571bb",
                                    "0x0",
                                    "0x3",
                                    "0x3",
                                    "0x69d2a153ec044578f0b4fa937c8da39cacdd0eb94a46c79f611435ac387c8ea",
                                    "0x12d7a52791baab55582f7d3ae7c9ca0b0629a4af8eedfb10da24b4a3c5fc0ff",
                                    "0x69c6162cbe927fca691b6ad3258e278e0c5c8bf732d7989fb78ada7f1ab10aa"
                                ],
                                "signature": [
                                    "0x5e92a65600cd637b70989b8f4f7a4eee029c35dbbb9a7c854a483344bb354c6",
                                    "0x7449bdb6aad1d24ca42dfe0e1af6fc3e15b3dd902162ccd305478a8cf6b315b"
                                ],
                                "transaction_hash": "0x2a39da1b0c5a023c76d958f09f3a246155515dd876eb127e2ffda5da10e5887",
                                "max_fee": "0x2386f26fc10000",
                                "nonce": "0x9",
                                "version": "0x1"
                            },
                            {
                                "type": "INVOKE_FUNCTION",
                                "sender_address": "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                "calldata": [
                                    "0x2",
                                    "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037",
                                    "0x169f135eddda5ab51886052d777a57f2ea9c162d713691b5e04a6d4ed71d47f",
                                    "0x0",
                                    "0x5",
                                    "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037",
                                    "0x2913ee03e5e3308c41e308bd391ea4faac9b9cb5062c76a6b3ab4f65397e106",
                                    "0x5",
                                    "0xf",
                                    "0x14",
                                    "0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0",
                                    "0xb07e8af891f5679e96de4249ee1a12c53c6f452e04fa7e7beb70ae1db2d6e9",
                                    "0x2",
                                    "0x4db652a9af00e11cc04e7ca6bfc656beab4327e65b3fe01ab1b1139d73d2033",
                                    "0x446bc71b024799e5e1add6d76038177b438affb051d5527cbb885702d82c36a",
                                    "0xa",
                                    "0x6335cb5f37a5a7c1b61eeb285e94572c41be2323f8a169ed96b5c5d554e006b",
                                    "0x350deec9a92a8b4fbab8b0358229b6e356eb47ecd6b20252989d11049ad8c92",
                                    "0x6d7d6bd574e8e5ca92791fc833ee8954bf0ad80ddf314666e2852d8e8e711ce",
                                    "0x4db90cf8d7cb322f5403805b0a480ebb7aa3a8376a0aed54d3ec5781a659b4a",
                                    "0x1d8d3081c44690ab24d6d0fdc1ef6786e6874a6dc1cc24c668c7b99860d200a",
                                    "0x411d5af97a7320ee2527a4cdec496ad4cd0b2c1d7c2b54e330a903547dca3b0",
                                    "0x7210b96a74b79b410064bdc2f1fe1840c7fca2bddd3ef34f1f62b2ae60b28bd",
                                    "0x5fef6edf3d6d7c8d101f6f78f02faa49eeb4fd8c25a6585a7bd6bbfc1ed08e0",
                                    "0x207f606ccf183a11d5dc9d9ad34fba6263cfd7de293d6ac1cf9a2f8de43b9e6",
                                    "0x51f637ddd507deb2942abd18de707ff4995788d18c4822c3a744dc974b13c68",
                                    "0x3",
                                    "0x20d587a4d69316089b2b2cf831ab033c7ad23f53aaebee8b11d5dd9a4e68d4b",
                                    "0x21aa9999e50ef163e5de2e9902b4ac6cc518293763def58a1e3f4ae14b74e9f",
                                    "0x6c0bd2259e9427b4e8edccad9b4bc2fcfebceab3ea4dc9c7bd026964e0fd546"
                                ],
                                "signature": [
                                    "0x1c0ade8477039c714f8e33267d4fdfe562f2b7bd95e6426b3f8d2e3d60a1b0a",
                                    "0x2ec477838d00f5f2cdfb87f6a8edc49eb88a1146ad17b29da1b095195751e69"
                                ],
                                "transaction_hash": "0x53f4692eff3d9585b6d2cef66f49143aa64b343aadd95b1a8d0807cb6d8345",
                                "max_fee": "0x2386f26fc10000",
                                "nonce": "0xa",
                                "version": "0x1"
                            },
                            {
                                "type": "INVOKE_FUNCTION",
                                "sender_address": "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                "calldata": [
                                    "0x2",
                                    "0x267311365224e8d4eb4dd580f1b737f990dfc81112ca71ecce147e774bcecb",
                                    "0xf818e4530ec36b83dfe702489b4df537308c3b798b0cc120e32c2056d68b7d",
                                    "0x0",
                                    "0x0",
                                    "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037",
                                    "0x169f135eddda5ab51886052d777a57f2ea9c162d713691b5e04a6d4ed71d47f",
                                    "0x0",
                                    "0x5",
                                    "0x5",
                                    "0x78401746828463e2c3f92ebb261fc82f7d4d4c8d9a80a356c44580dab124cb0",
                                    "0xbd3f0c0695cbe60e5f3bef8427f3fe21501f08ff15e6dee59ab7e3a684e1ad",
                                    "0x2",
                                    "0x2ba1e9106786fe90b2666d877ed7c15238ea399dccb14d490dec118d676a02e",
                                    "0x7c87e32bae977db270b344ec731df28b8a91bf033c4c73c028215e513a190b0"
                                ],
                                "signature": [
                                    "0x1cacd47074ae6a3cb38acd5f1f23376fc0b636b7e3dcd247e1a4c91159eb446",
                                    "0x3f847323dbf82ccc03f32ebc4ffe92a6a33824211436d44285e7f5bb5a1b48a"
                                ],
                                "transaction_hash": "0x2006692968a388cbd285946faa9a26198e2d7d2eb7f5c1b93e58593bc7d16b9",
                                "max_fee": "0x2386f26fc10000",
                                "nonce": "0xb",
                                "version": "0x1"
                            }
                        ],
                        "transaction_receipts": [
                            {
                                "transaction_hash": "0x2a39da1b0c5a023c76d958f09f3a246155515dd876eb127e2ffda5da10e5887",
                                "transaction_index": 0,
                                "execution_status": "SUCCEEDED",
                                "execution_resources": {
                                    "n_steps": 4819,
                                    "n_memory_holes": 0,
                                    "builtin_instance_counter": {
                                        "pedersen_builtin": 18,
                                        "range_check_builtin": 109,
                                        "bitwise_builtin": 2,
                                        "output_builtin": null,
                                        "ecdsa_builtin": 1,
                                        "ec_op_builtin": null,
                                        "poseidon_builtin": null,
                                        "keccak_builtin": null,
                                        "segment_arena_builtin": null
                                    },
                                    "data_availability": null,
                                    "total_gas_consumed": null
                                },
                                "l1_to_l2_consumed_message": null,
                                "l2_to_l1_messages": [],
                                "events": [
                                    {
                                        "from_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                                        "keys": [
                                            "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                                        ],
                                        "data": [
                                            "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                            "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                                            "0x62b936e2c7a",
                                            "0x0"
                                        ]
                                    }
                                ],
                                "actual_fee": "0x62b936e2c7a"
                            },
                            {
                                "transaction_hash": "0x53f4692eff3d9585b6d2cef66f49143aa64b343aadd95b1a8d0807cb6d8345",
                                "transaction_index": 1,
                                "execution_status": "SUCCEEDED",
                                "execution_resources": {
                                    "n_steps": 6326,
                                    "n_memory_holes": 0,
                                    "builtin_instance_counter": {
                                        "pedersen_builtin": 23,
                                        "range_check_builtin": 143,
                                        "bitwise_builtin": null,
                                        "output_builtin": null,
                                        "ecdsa_builtin": 1,
                                        "ec_op_builtin": null,
                                        "poseidon_builtin": null,
                                        "keccak_builtin": null,
                                        "segment_arena_builtin": null
                                    },
                                    "data_availability": null,
                                    "total_gas_consumed": null
                                },
                                "l1_to_l2_consumed_message": null,
                                "l2_to_l1_messages": [],
                                "events": [
                                    {
                                        "from_address": "0xa3d19d9e80d74dd6140fed379e2c10a21609374811b244cc9d7d1f6d9e0037",
                                        "keys": [
                                            "0x6335cb5f37a5a7c1b61eeb285e94572c41be2323f8a169ed96b5c5d554e006b",
                                            "0x350deec9a92a8b4fbab8b0358229b6e356eb47ecd6b20252989d11049ad8c92",
                                            "0x6d7d6bd574e8e5ca92791fc833ee8954bf0ad80ddf314666e2852d8e8e711ce",
                                            "0x4db90cf8d7cb322f5403805b0a480ebb7aa3a8376a0aed54d3ec5781a659b4a",
                                            "0x1d8d3081c44690ab24d6d0fdc1ef6786e6874a6dc1cc24c668c7b99860d200a",
                                            "0x411d5af97a7320ee2527a4cdec496ad4cd0b2c1d7c2b54e330a903547dca3b0",
                                            "0x7210b96a74b79b410064bdc2f1fe1840c7fca2bddd3ef34f1f62b2ae60b28bd",
                                            "0x5fef6edf3d6d7c8d101f6f78f02faa49eeb4fd8c25a6585a7bd6bbfc1ed08e0",
                                            "0x207f606ccf183a11d5dc9d9ad34fba6263cfd7de293d6ac1cf9a2f8de43b9e6",
                                            "0x51f637ddd507deb2942abd18de707ff4995788d18c4822c3a744dc974b13c68"
                                        ],
                                        "data": [
                                            "0x20d587a4d69316089b2b2cf831ab033c7ad23f53aaebee8b11d5dd9a4e68d4b",
                                            "0x21aa9999e50ef163e5de2e9902b4ac6cc518293763def58a1e3f4ae14b74e9f",
                                            "0x6c0bd2259e9427b4e8edccad9b4bc2fcfebceab3ea4dc9c7bd026964e0fd546"
                                        ]
                                    },
                                    {
                                        "from_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                                        "keys": [
                                            "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                                        ],
                                        "data": [
                                            "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                            "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                                            "0x592b6b617a8",
                                            "0x0"
                                        ]
                                    }
                                ],
                                "actual_fee": "0x592b6b617a8"
                            },
                            {
                                "transaction_hash": "0x2006692968a388cbd285946faa9a26198e2d7d2eb7f5c1b93e58593bc7d16b9",
                                "transaction_index": 2,
                                "execution_status": "SUCCEEDED",
                                "execution_resources": {
                                    "n_steps": 7935,
                                    "n_memory_holes": 0,
                                    "builtin_instance_counter": {
                                        "pedersen_builtin": 23,
                                        "range_check_builtin": 245,
                                        "bitwise_builtin": 6,
                                        "output_builtin": null,
                                        "ecdsa_builtin": 1,
                                        "ec_op_builtin": null,
                                        "poseidon_builtin": null,
                                        "keccak_builtin": 1,
                                        "segment_arena_builtin": null
                                    },
                                    "data_availability": null,
                                    "total_gas_consumed": null
                                },
                                "l1_to_l2_consumed_message": null,
                                "l2_to_l1_messages": [],
                                "events": [
                                    {
                                        "from_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                                        "keys": [
                                            "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                                        ],
                                        "data": [
                                            "0x60664b576dae484dc3430ed3b1036e7879712e2c2c2728f568b8dbcbbc0f655",
                                            "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                                            "0x596cf7e9048",
                                            "0x0"
                                        ]
                                    }
                                ],
                                "actual_fee": "0x596cf7e9048"
                            }
                        ],
                        "starknet_version": "0.12.3"
                    }
                }"#);
        });

        println!("Starting task");

        // Call l2_fetch_task with a timeout
        let task = tokio::spawn(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                l2_fetch_task(backend, 0, Some(1), fetch_stream_sender, provider, None, once_caught_up_sender),
            )
            .await
        });

        println!("Waiting for blocks");

        // Assert that 1 block was received (since we're only mocking one block now)
        match tokio::time::timeout(std::time::Duration::from_secs(5), fetch_stream_receiver.recv()).await {
            Ok(Some(block)) => {
                assert_eq!(block.unverified_block_number, Some(27));
                println!("Received block 27");
            }
            Ok(None) => panic!("Channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for block"),
        }

        // Assert that once_caught_up_callback was triggered
        match tokio::time::timeout(std::time::Duration::from_secs(1), once_caught_up_receiver).await {
            Ok(Ok(())) => println!("Caught up callback received"),
            Ok(Err(_)) => panic!("Caught up channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for caught up callback"),
        }

        // Ensure the task completed successfully
        match task.await {
            Ok(Ok(Ok(()))) => println!("Task completed successfully"),
            Ok(Ok(Err(e))) => panic!("Task failed with error: {:?}", e),
            Ok(Err(_)) => panic!("Task timed out"),
            Err(e) => panic!("Task panicked: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_catch_up_to_chain_tip() {
        // TODO: Implement test for catching up to chain tip
        // - Mock server to return BlockNotFound after X blocks
        // - Call l2_fetch_task with n_blocks_to_sync set to None
        // - Assert that the function stops after receiving BlockNotFound
        // - Assert that once_caught_up_callback was triggered
    }

    #[tokio::test]
    async fn test_polling_behavior() {
        // TODO: Implement test for polling behavior
        // - Mock server to return BlockNotFound after X blocks, then return new blocks
        // - Call l2_fetch_task with a small sync_polling_interval
        // - Assert that function continues to fetch new blocks after initial catch-up
        // - Assert that once_caught_up_callback was triggered only once
    }

    #[tokio::test]
    async fn test_error_handling() {
        // TODO: Implement test for error handling
        // - Mock server to occasionally return errors
        // - Call l2_fetch_task and observe its behavior
        // - Assert that it correctly handles and potentially retries on errors
        // - Assert that it doesn't crash on recoverable errors
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        // TODO: Implement test for graceful shutdown
        // - Start l2_fetch_task in a separate task
        // - Trigger a graceful shutdown signal
        // - Assert that the function terminates cleanly
    }

    #[tokio::test]
    async fn test_large_number_of_blocks() {
        // TODO: Implement test for fetching a large number of blocks
        // - Mock server to return many blocks
        // - Call l2_fetch_task with n_blocks_to_sync set to a large number (e.g., 10000)
        // - Assert that all blocks are fetched correctly
        // - Monitor memory usage and performance
    }

    #[tokio::test]
    async fn test_parallel_fetching() {
        // TODO: Implement test for parallel fetching
        // - Mock server to add a delay to each block fetch
        // - Call l2_fetch_task with n_blocks_to_sync set to a moderate number
        // - Measure the total time taken
        // - Assert that the time taken is significantly less than sequential fetching
    }

    #[tokio::test]
    async fn test_channel_backpressure() {
        // TODO: Implement test for channel backpressure
        // - Create a slow receiver for fetch_stream_sender
        // - Call l2_fetch_task with a fast mock server
        // - Assert that the function handles backpressure correctly
    }

    #[tokio::test]
    async fn test_provider_errors() {
        // TODO: Implement test for various provider errors
        // - Mock server to return different types of errors
        // - Call l2_fetch_task multiple times with different error scenarios
        // - Assert that each error type is handled appropriately
    }

    #[tokio::test]
    async fn test_resume_from_specific_block() {
        // TODO: Implement test for resuming from a specific block
        // - Call l2_fetch_task with first_block set to a non-zero value
        // - Assert that fetching starts from the specified block
    }
}
