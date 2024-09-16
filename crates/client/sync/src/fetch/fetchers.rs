//! Contains the code required to fetch data from the network efficiently.
use super::FetchError;
use crate::l2::L2SyncError;
use anyhow::Context;
use core::fmt;
use core::time::Duration;
use futures::FutureExt;
use mc_block_import::{UnverifiedCommitments, UnverifiedFullBlock, UnverifiedHeader, UnverifiedPendingFullBlock};
use mp_block::header::GasPrices;
use mp_chain_config::StarknetVersion;
use mp_class::class_update::{ClassUpdate, LegacyClassUpdate, SierraClassUpdate};
use mp_class::MISSED_CLASS_HASHES;
use mp_convert::{felt_to_u128, ToFelt};
use mp_receipt::TransactionReceipt;
use mp_transactions::{Transaction, MAIN_CHAIN_ID};
use mp_utils::{stopwatch_end, wait_or_graceful_shutdown, PerfStopwatch};
use starknet_api::core::ChainId;
use starknet_core::types::{ContractClass, StarknetError};
use starknet_providers::sequencer::models::{Block as SequencerBlock, StateUpdate as SequencerStateUpdate};
use starknet_providers::{Provider, ProviderError, SequencerGatewayProvider};
use starknet_types_core::felt::Felt;
use url::Url;

const MAX_RETRY: u32 = 15;
const BASE_DELAY: Duration = Duration::from_secs(1);

/// The configuration of the worker responsible for fetching new blocks and state updates from the
/// feeder.
#[derive(Clone, Debug)]
pub struct FetchConfig {
    /// The URL of the sequencer gateway.
    pub gateway: Url,
    /// The URL of the feeder gateway.
    pub feeder_gateway: Url,
    /// The ID of the chain served by the sequencer gateway.
    pub chain_id: ChainId,
    /// Whether to play a sound when a new block is fetched.
    pub sound: bool,
    /// Whether to check the root of the state update.
    pub verify: bool,
    /// The optional API_KEY to avoid rate limiting from the sequencer gateway.
    pub api_key: Option<String>,
    /// Polling interval.
    pub sync_polling_interval: Option<Duration>,
    /// Number of blocks to sync (for testing purposes).
    pub n_blocks_to_sync: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FetchBlockId {
    BlockN(u64),
    Pending,
}

impl FetchBlockId {
    pub fn block_n(self) -> Option<u64> {
        match self {
            FetchBlockId::BlockN(block_n) => Some(block_n),
            FetchBlockId::Pending => None,
        }
    }
}

impl fmt::Debug for FetchBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlockN(block) => write!(f, "#{}", block),
            Self::Pending => write!(f, "<pending>"),
        }
    }
}

impl From<FetchBlockId> for starknet_providers::sequencer::models::BlockId {
    fn from(value: FetchBlockId) -> Self {
        match value {
            FetchBlockId::BlockN(block_n) => starknet_providers::sequencer::models::BlockId::Number(block_n),
            FetchBlockId::Pending => starknet_providers::sequencer::models::BlockId::Pending,
        }
    }
}
impl From<FetchBlockId> for starknet_core::types::BlockId {
    fn from(value: FetchBlockId) -> Self {
        match value {
            FetchBlockId::BlockN(block_n) => starknet_core::types::BlockId::Number(block_n),
            FetchBlockId::Pending => starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending),
        }
    }
}

pub async fn fetch_pending_block_and_updates(
    chain_id: &ChainId,
    provider: &SequencerGatewayProvider,
) -> Result<UnverifiedPendingFullBlock, FetchError> {
    let block_id = FetchBlockId::Pending;

    let sw = PerfStopwatch::new();
    let (state_update, block) = retry(
        || async {
            let (state_update, block) = fetch_state_update_with_block(provider, block_id).await?;
            if let Some(block_hash) = block.block_hash {
                // HACK: Apparently the FGW sometimes returns a closed block when fetching the pending block. Interesting..?
                log::debug!(
                    "Fetched a pending block, got a closed one: block_number={:?} block_hash={:#x}",
                    block.block_number,
                    block_hash,
                );
                Err(ProviderError::StarknetError(StarknetError::BlockNotFound))
            } else {
                Ok((state_update, block))
            }
        },
        MAX_RETRY,
        BASE_DELAY,
    )
    .await?;

    let class_update = fetch_class_updates(chain_id, &state_update, block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    println!("block = {:?}", &(&block, &state_update, &class_update));
    let converted = convert_sequencer_pending_block(block, state_update, class_update)
        .context("Parsing the FGW pending block format")?;
    println!("converted = {}", serde_json::to_string_pretty(&converted).unwrap());
    Ok(converted)
}

pub async fn fetch_block_and_updates(
    chain_id: &ChainId,
    block_n: u64,
    provider: &SequencerGatewayProvider,
) -> Result<UnverifiedFullBlock, FetchError> {
    let block_id = FetchBlockId::BlockN(block_n);

    let sw = PerfStopwatch::new();
    let (state_update, block) =
        retry(|| fetch_state_update_with_block(provider, block_id), MAX_RETRY, BASE_DELAY).await?;
    let class_update = fetch_class_updates(chain_id, &state_update, block_id, provider).await?;

    stopwatch_end!(sw, "fetching {:?}: {:?}", block_id);

    let converted =
        convert_sequencer_block(block, state_update, class_update).context("Parsing the FGW full block format")?;
    Ok(converted)
}

async fn retry<F, Fut, T>(mut f: F, max_retries: u32, base_delay: Duration) -> Result<T, ProviderError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ProviderError>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(res) => return Ok(res),
            Err(ProviderError::StarknetError(StarknetError::BlockNotFound)) => {
                break Err(ProviderError::StarknetError(StarknetError::BlockNotFound));
            }
            Err(err) => {
                let delay = base_delay * 2_u32.pow(attempt).min(6); // Cap to prevent overly long delays
                attempt += 1;
                if attempt > max_retries {
                    break Err(err);
                }
                match err {
                    ProviderError::RateLimited => {
                        log::info!("The fetching process has been rate limited, retrying in {:?}", delay)
                    }
                    _ => log::warn!("The provider has returned an error: {}, retrying in {:?}", err, delay),
                }
                if wait_or_graceful_shutdown(tokio::time::sleep(delay)).await.is_none() {
                    return Err(ProviderError::StarknetError(StarknetError::BlockNotFound));
                    // :/
                }
            }
        }
    }
}

/// retrieves state update with block from Starknet sequencer in only one request
async fn fetch_state_update_with_block(
    provider: &SequencerGatewayProvider,
    block_id: FetchBlockId,
) -> Result<
    (starknet_providers::sequencer::models::StateUpdate, starknet_providers::sequencer::models::Block),
    ProviderError,
> {
    #[allow(deprecated)] // Sequencer-specific functions are deprecated. Use it via the Provider trait instead.
    let state_update_with_block = provider.get_state_update_with_block(block_id.into()).await?;

    Ok((state_update_with_block.state_update, state_update_with_block.block))
}

/// retrieves class updates from Starknet sequencer
async fn fetch_class_updates(
    chain_id: &ChainId,
    state_update: &starknet_providers::sequencer::models::StateUpdate,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> anyhow::Result<Vec<ClassUpdate>> {
    let chain_id: Felt = chain_id.to_felt();

    // for blocks before 2597 on mainnet new classes are not declared in the state update
    // https://github.com/madara-alliance/madara/issues/233
    let legacy_classes: Vec<_> = if chain_id == MAIN_CHAIN_ID && block_id.block_n().is_some_and(|id| id < 2597) {
        let block_number = block_id.block_n().unwrap(); // Safe to unwrap because of the condition above
        MISSED_CLASS_HASHES.get(&block_number).cloned().unwrap_or_default()
    } else {
        state_update.state_diff.old_declared_contracts.clone()
    };

    let sierra_classes: Vec<_> = state_update
        .state_diff
        .declared_classes
        .iter()
        .map(|declared_class| (declared_class.class_hash, &declared_class.compiled_class_hash))
        .collect();

    let legacy_class_futures = legacy_classes.into_iter().map(|class_hash| {
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id, provider), 15, Duration::from_secs(1)).await?;

            let starknet_core::types::ContractClass::Legacy(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };

            Ok::<_, L2SyncError>(ClassUpdate::Legacy(LegacyClassUpdate { class_hash, contract_class }))
        }
        .boxed()
    });

    let sierra_class_futures = sierra_classes.into_iter().map(|(class_hash, &compiled_class_hash)| {
        async move {
            let (class_hash, contract_class) =
                retry(|| fetch_class(class_hash, block_id, provider), 15, Duration::from_secs(1)).await?;

            let starknet_core::types::ContractClass::Sierra(contract_class) = contract_class else {
                return Err(L2SyncError::UnexpectedClassType { class_hash });
            };

            Ok::<_, L2SyncError>(ClassUpdate::Sierra(SierraClassUpdate {
                class_hash,
                contract_class,
                compiled_class_hash,
            }))
        }
        .boxed()
    });

    Ok(futures::future::try_join_all(legacy_class_futures.chain(sierra_class_futures)).await?)
}

/// Downloads a class definition from the Starknet sequencer. Note that because
/// of the current type hell we decided to deal with raw JSON data instead of starknet-providers `DeployedContract`.
async fn fetch_class(
    class_hash: Felt,
    block_id: FetchBlockId,
    provider: &SequencerGatewayProvider,
) -> Result<(Felt, ContractClass), ProviderError> {
    let contract_class = provider.get_class(starknet_core::types::BlockId::from(block_id), class_hash).await?;
    log::debug!("Got the contract class {:?}", contract_class);
    Ok((class_hash, contract_class))
}

fn convert_block_header(block: &SequencerBlock) -> anyhow::Result<UnverifiedHeader> {
    println!("{:?}", block.starknet_version);
    Ok(UnverifiedHeader {
        parent_block_hash: Some(block.parent_block_hash),
        sequencer_address: block.sequencer_address.unwrap_or_default(),
        block_timestamp: block.timestamp,
        protocol_version: block
            .starknet_version
            .as_deref()
            .map(|v| v.parse().context("Invalid starknet version"))
            .unwrap_or_else(|| {
                StarknetVersion::try_from_mainnet_block_number(
                    block.block_number.context("A block number is needed to determine the missing Starknet version")?,
                )
                .context("Unable to determine the Starknet version")
            })?,
        l1_gas_price: GasPrices {
            eth_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_wei).context("Converting prices")?,
            strk_l1_gas_price: felt_to_u128(&block.l1_gas_price.price_in_fri).context("Converting prices")?,
            eth_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_wei).context("Converting prices")?,
            strk_l1_data_gas_price: felt_to_u128(&block.l1_data_gas_price.price_in_fri).context("Converting prices")?,
        },
        l1_da_mode: block.l1_da_mode.into(),
    })
}

fn convert_sequencer_pending_block(
    block: SequencerBlock,
    state_update: SequencerStateUpdate,
    class_update: Vec<ClassUpdate>,
) -> anyhow::Result<UnverifiedPendingFullBlock> {
    Ok(UnverifiedPendingFullBlock {
        header: convert_block_header(&block)?,
        state_diff: state_update.state_diff.into(),
        receipts: block
            .transaction_receipts
            .into_iter()
            .zip(&block.transactions)
            .map(|(receipt, tx)| TransactionReceipt::from_provider(receipt, tx))
            .collect(),
        transactions: block
            .transactions
            .into_iter()
            .map(Transaction::try_from)
            .collect::<Result<_, _>>()
            .context("Converting the transactions")?,
        declared_classes: class_update.into_iter().map(Into::into).collect(),
    })
}

fn convert_sequencer_block(
    block: SequencerBlock,
    state_update: SequencerStateUpdate,
    class_update: Vec<ClassUpdate>,
) -> anyhow::Result<UnverifiedFullBlock> {
    // Verify against these commitments.
    let commitments = UnverifiedCommitments {
        // TODO: these commitments are wrong for mainnet from block 0 to unknown. We need to figure out
        // which blocks and handle the case directly in the block import crate.
        // transaction_commitment: Some(block.transaction_commitment.context("No transaction commitment")?),
        // event_commitment: Some(block.event_commitment.context("No event commitment")?),
        state_diff_commitment: None,
        receipt_commitment: None,
        global_state_root: Some(block.state_root.context("No state root")?),
        block_hash: Some(block.block_hash.context("No block hash")?),
        ..Default::default()
    };
    Ok(UnverifiedFullBlock {
        unverified_block_number: Some(
            block.block_number.context("FGW should return a block number for closed blocks")?,
        ),
        header: convert_block_header(&block)?,
        state_diff: state_update.state_diff.into(),
        receipts: block
            .transaction_receipts
            .into_iter()
            .zip(&block.transactions)
            .map(|(receipt, tx)| TransactionReceipt::from_provider(receipt, tx))
            .collect(),
        transactions: block
            .transactions
            .into_iter()
            .map(Transaction::try_from)
            .collect::<Result<_, _>>()
            .context("Converting the transactions")?,
        declared_classes: class_update.into_iter().map(Into::into).collect(),
        commitments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use starknet_core::types::{DataAvailabilityResources, L1DataAvailabilityMode, ResourcePrice};
    use starknet_providers::sequencer::models::{
        state_update::{StateDiff, StorageDiff},
        BlockStatus, BuiltinInstanceCounter, ConfirmedTransactionReceipt, Event, ExecutionResources,
        InvokeFunctionTransaction, TransactionExecutionStatus, TransactionType,
    };

    #[fixture]
    fn sequencer_gateway_provider() -> SequencerGatewayProvider {
        SequencerGatewayProvider::starknet_alpha_mainnet()
    }

    #[rstest]
    #[tokio::test]
    async fn test_can_fetch_pending_block(sequencer_gateway_provider: SequencerGatewayProvider) {
        let block = fetch_pending_block_and_updates(&ChainId::Mainnet, &sequencer_gateway_provider).await.unwrap();
        // ignore as we can't check much here :/
        drop(block);
    }

    #[rstest]
    #[tokio::test]
    async fn test_can_convert_pending_block() {
        let felt = |s: &str| Felt::from_hex_unchecked(s);

        let block = (
            SequencerBlock {
                block_hash: None,
                block_number: None,
                parent_block_hash: felt("0x6c3b6c94d4add68c0309218708baa09463887f51863115346d380d67269a68a"),
                timestamp: 1726480207,
                sequencer_address: Some(felt("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8")),
                state_root: None,
                transaction_commitment: None,
                event_commitment: None,
                status: BlockStatus::Pending,
                l1_da_mode: L1DataAvailabilityMode::Calldata,
                l1_gas_price: ResourcePrice { price_in_fri: felt("0x993f2ef1b9f1"), price_in_wei: felt("0x6894e3020") },
                l1_data_gas_price: ResourcePrice {
                    price_in_fri: felt("0x375526f9a8f0f"),
                    price_in_wei: felt("0x25c2d9f043"),
                },
                transactions: vec![
                    TransactionType::InvokeFunction(InvokeFunctionTransaction {
                        sender_address: felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                        entry_point_selector: None,
                        calldata: vec![
                            felt("0x2"),
                            felt("0x0"),
                            felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                        ],
                        signature: vec![felt("0x3b81b87ad249504cd88f031c0b1c7a893eedb8bce50ca88c53a051014d0a25d")],
                        transaction_hash: felt("0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858"),
                        max_fee: Some(felt("0x3f6c4a9a01d28")),
                        nonce: Some(felt("0x12")),
                        nonce_data_availability_mode: None,
                        fee_data_availability_mode: None,
                        resource_bounds: None,
                        tip: None,
                        paymaster_data: None,
                        account_deployment_data: None,
                        version: felt("0x1"),
                    }),
                    TransactionType::InvokeFunction(InvokeFunctionTransaction {
                        sender_address: felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"),
                        entry_point_selector: None,
                        calldata: vec![
                            felt("0x2"),
                            felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                        ],
                        signature: vec![
                            felt("0x16337397dd75f3d7f95bcca3509149b266c90ab5fdf87c7d950767b78fab231"),
                            felt("0x7bcc57dfc8159fa54de2d2e87ea6ecd4204e7cbc2173d0b2345d43c4028f944"),
                        ],
                        transaction_hash: felt("0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8"),
                        max_fee: Some(felt("0x3f6c4a9191bc1")),
                        nonce: Some(felt("0x1b")),
                        nonce_data_availability_mode: None,
                        fee_data_availability_mode: None,
                        resource_bounds: None,
                        tip: None,
                        paymaster_data: None,
                        account_deployment_data: None,
                        version: felt("0x1"),
                    }),
                ],
                transaction_receipts: vec![
                    ConfirmedTransactionReceipt {
                        transaction_hash: felt("0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858"),
                        transaction_index: 0,
                        execution_status: Some(TransactionExecutionStatus::Succeeded),
                        revert_error: None,
                        execution_resources: Some(ExecutionResources {
                            n_steps: 87793,
                            n_memory_holes: 0,
                            builtin_instance_counter: BuiltinInstanceCounter {
                                pedersen_builtin: Some(114),
                                range_check_builtin: Some(9393),
                                bitwise_builtin: Some(12),
                                output_builtin: None,
                                ecdsa_builtin: None,
                                ec_op_builtin: Some(6),
                                poseidon_builtin: None,
                                keccak_builtin: None,
                                segment_arena_builtin: None,
                            },
                            data_availability: Some(DataAvailabilityResources { l1_gas: 12838, l1_data_gas: 0 }),
                            total_gas_consumed: Some(DataAvailabilityResources { l1_gas: 13233, l1_data_gas: 0 }),
                        }),
                        l1_to_l2_consumed_message: None,
                        l2_to_l1_messages: vec![],
                        events: vec![
                            Event {
                                from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                                keys: vec![
                                    felt("0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff"),
                                    felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                    felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                                ],
                                data: vec![felt("0x7738106d5"), felt("0x0")],
                            },
                            Event {
                                from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                                keys: vec![
                                    felt("0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff"),
                                    felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                    felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                                ],
                                data: vec![felt("0x0"), felt("0x0")],
                            },
                            Event {
                                from_address: felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                                keys: vec![
                                    felt("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"),
                                    felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"),
                                    felt("0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"),
                                ],
                                data: vec![felt("0x7738106d5"), felt("0x0")],
                            },
                        ],
                        actual_fee: felt("0x151df82a5a620"),
                    },
                    ConfirmedTransactionReceipt {
                        transaction_hash: felt("0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8"),
                        transaction_index: 1,
                        execution_status: Some(TransactionExecutionStatus::Succeeded),
                        revert_error: None,
                        execution_resources: Some(ExecutionResources {
                            n_steps: 59639,
                            n_memory_holes: 0,
                            builtin_instance_counter: BuiltinInstanceCounter {
                                pedersen_builtin: Some(110),
                                range_check_builtin: Some(4129),
                                bitwise_builtin: Some(12),
                                output_builtin: None,
                                ecdsa_builtin: None,
                                ec_op_builtin: Some(6),
                                poseidon_builtin: None,
                                keccak_builtin: None,
                                segment_arena_builtin: None,
                            },
                            data_availability: Some(DataAvailabilityResources { l1_gas: 11736, l1_data_gas: 0 }),
                            total_gas_consumed: Some(DataAvailabilityResources { l1_gas: 11921, l1_data_gas: 0 }),
                        }),
                        l1_to_l2_consumed_message: None,
                        l2_to_l1_messages: vec![],
                        events: vec![Event {
                            from_address: felt("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
                            keys: vec![felt("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9")],
                            data: vec![
                                felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"),
                                felt("0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8"),
                                felt("0x1305fd1ef0220"),
                                felt("0x0"),
                            ],
                        }],
                        actual_fee: felt("0x1305fd1ef0220"),
                    },
                ],
                starknet_version: Some("0.13.2.1".into()),
            },
            SequencerStateUpdate {
                block_hash: None,
                new_root: None,
                old_root: felt("0x7b664bb7e0de32440cb925f3b0580a3f72d42442bd508fbe44f78956baec413"),
                state_diff: StateDiff {
                    storage_diffs: [
                        (
                            felt("0x13e48fb52751595411acd0153a04be2f9472e23cc06eb52dd991145914234ec"),
                            vec![
                                StorageDiff {
                                    key: felt("0x12f1312f6092a30a200f21c6e298f5a94b3522583fe40c98c035b7fd8049b0"),
                                    value: felt("0x25b0807ac00"),
                                },
                                StorageDiff {
                                    key: felt("0x1d411efc1b91ba8ec75807861b5ba3fc72f0df8d68a1155a6b4c6717865364"),
                                    value: felt("0x3dc5cb078c05c00"),
                                },
                            ],
                        ),
                        (
                            felt("0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"),
                            vec![StorageDiff {
                                key: felt("0xbc72301fcd3b05852cc058ab4cae654f59901211906ce8eaab14eff5546aae"),
                                value: felt("0x0"),
                            }],
                        ),
                    ]
                    .into(),
                    deployed_contracts: vec![],
                    old_declared_contracts: vec![],
                    declared_classes: vec![],
                    replaced_classes: vec![],
                    nonces: [
                        (felt("0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"), felt("0x13")),
                        (felt("0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8"), felt("0x1c")),
                        (felt("0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535"), felt("0x3af1")),
                    ]
                    .into(),
                },
            },
            vec![],
        );
        let converted = convert_sequencer_pending_block(block.0, block.1, block.2).unwrap();
        println!("{}", serde_json::to_string_pretty(&converted).unwrap());
        assert_eq!(
            converted,
            serde_json::from_value(serde_json::json!({
              "header": {
                "parent_block_hash": "0x6c3b6c94d4add68c0309218708baa09463887f51863115346d380d67269a68a",
                "sequencer_address": "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                "block_timestamp": 1726480207,
                "protocol_version": [
                  0,
                  13,
                  2,
                  1
                ],
                "l1_gas_price": {
                  "eth_l1_gas_price": 28073406496u64,
                  "strk_l1_gas_price": 168496649583089u64,
                  "eth_l1_data_gas_price": 162182852675u64,
                  "strk_l1_data_gas_price": 973421850300175u64
                },
                "l1_da_mode": "Calldata"
              },
              "state_diff": {
                "storage_diffs": [
                  {
                    "address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                    "storage_entries": [
                      {
                        "key": "0xbc72301fcd3b05852cc058ab4cae654f59901211906ce8eaab14eff5546aae",
                        "value": "0x0"
                      }
                    ]
                  },
                  {
                    "address": "0x13e48fb52751595411acd0153a04be2f9472e23cc06eb52dd991145914234ec",
                    "storage_entries": [
                      {
                        "key": "0x12f1312f6092a30a200f21c6e298f5a94b3522583fe40c98c035b7fd8049b0",
                        "value": "0x25b0807ac00"
                      },
                      {
                        "key": "0x1d411efc1b91ba8ec75807861b5ba3fc72f0df8d68a1155a6b4c6717865364",
                        "value": "0x3dc5cb078c05c00"
                      }
                    ]
                  }
                ],
                "deprecated_declared_classes": [],
                "declared_classes": [],
                "deployed_contracts": [],
                "replaced_classes": [],
                "nonces": [
                  {
                    "contract_address": "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                    "nonce": "0x13"
                  },
                  {
                    "contract_address": "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                    "nonce": "0x1c"
                  },
                  {
                    "contract_address": "0x2bb8a1f5a1241c1ebe8e10ff93b38ab097b1a20f77517997f8799829e096535",
                    "nonce": "0x3af1"
                  }
                ]
              },
              "transactions": [
                {
                  "Invoke": {
                    "V1": {
                      "sender_address": "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                      "calldata": [
                        "0x2",
                        "0x0",
                        "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1"
                      ],
                      "max_fee": "0x3f6c4a9a01d28",
                      "signature": [
                        "0x3b81b87ad249504cd88f031c0b1c7a893eedb8bce50ca88c53a051014d0a25d"
                      ],
                      "nonce": "0x12"
                    }
                  }
                },
                {
                  "Invoke": {
                    "V1": {
                      "sender_address": "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                      "calldata": [
                        "0x2",
                        "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996"
                      ],
                      "max_fee": "0x3f6c4a9191bc1",
                      "signature": [
                        "0x16337397dd75f3d7f95bcca3509149b266c90ab5fdf87c7d950767b78fab231",
                        "0x7bcc57dfc8159fa54de2d2e87ea6ecd4204e7cbc2173d0b2345d43c4028f944"
                      ],
                      "nonce": "0x1b"
                    }
                  }
                }
              ],
              "receipts": [
                {
                  "Invoke": {
                    "transaction_hash": "0x40a00bf7c2ccc154b4d18e63f072f69a0ce99c5249c2b7b62076dc017408858",
                    "actual_fee": {
                      "amount": "0x151df82a5a620",
                      "unit": "Wei"
                    },
                    "messages_sent": [],
                    "events": [
                      {
                        "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                        "keys": [
                          "0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff",
                          "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                          "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                        ],
                        "data": [
                          "0x7738106d5",
                          "0x0"
                        ]
                      },
                      {
                        "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                        "keys": [
                          "0x134692b230b9e1ffa39098904722134159652b09c5bc41d88d6698779d228ff",
                          "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                          "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                        ],
                        "data": [
                          "0x0",
                          "0x0"
                        ]
                      },
                      {
                        "from_address": "0x2cd937c3dccd4a4e125011bbe3189a6db0419bb6dd95c4b5ce5f6d834d8996",
                        "keys": [
                          "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9",
                          "0x13fbb0f1e4f795e070d6a54a618114df7d60b8e1e183afa4822db06835105b1",
                          "0x7f39fc9465588bb783023401230d2318354b23e71e632aa7019a423d284f8c4"
                        ],
                        "data": [
                          "0x7738106d5",
                          "0x0"
                        ]
                      }
                    ],
                    "execution_resources": {
                      "steps": 87793,
                      "memory_holes": 0,
                      "range_check_builtin_applications": 9393,
                      "pedersen_builtin_applications": 114,
                      "poseidon_builtin_applications": null,
                      "ec_op_builtin_applications": 6,
                      "ecdsa_builtin_applications": null,
                      "bitwise_builtin_applications": 12,
                      "keccak_builtin_applications": null,
                      "segment_arena_builtin": null,
                      "data_availability": {
                        "l1_gas": 12838,
                        "l1_data_gas": 0
                      },
                      "total_gas_consumed": {
                        "l1_gas": 13233,
                        "l1_data_gas": 0
                      }
                    },
                    "execution_result": "Succeeded"
                  }
                },
                {
                  "Invoke": {
                    "transaction_hash": "0x39441f846a99c8fec3cdb328bf00a9ee3d664f732f2c9649e324e61582762e8",
                    "actual_fee": {
                      "amount": "0x1305fd1ef0220",
                      "unit": "Wei"
                    },
                    "messages_sent": [],
                    "events": [
                      {
                        "from_address": "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7",
                        "keys": [
                          "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                        ],
                        "data": [
                          "0x1ac14b1f2986d5994afb8cbdc788f0659aaebb9a68a8f91306f60fe4856b2b8",
                          "0x1176a1bd84444c89232ec27754698e5d2e7e1a7f1539f12027f28b23ec9f3d8",
                          "0x1305fd1ef0220",
                          "0x0"
                        ]
                      }
                    ],
                    "execution_resources": {
                      "steps": 59639,
                      "memory_holes": 0,
                      "range_check_builtin_applications": 4129,
                      "pedersen_builtin_applications": 110,
                      "poseidon_builtin_applications": null,
                      "ec_op_builtin_applications": 6,
                      "ecdsa_builtin_applications": null,
                      "bitwise_builtin_applications": 12,
                      "keccak_builtin_applications": null,
                      "segment_arena_builtin": null,
                      "data_availability": {
                        "l1_gas": 11736,
                        "l1_data_gas": 0
                      },
                      "total_gas_consumed": {
                        "l1_gas": 11921,
                        "l1_data_gas": 0
                      }
                    },
                    "execution_result": "Succeeded"
                  }
                }
              ],
              "declared_classes": []
            }))
            .unwrap()
        )
    }

    // TODO: I'd like to have more tests for more starknt versions, but i don't want to commit multiple megabytes into the repository.
    #[rstest]
    #[case(0)]
    #[case(724_130)]
    #[tokio::test]
    async fn test_can_fetch_and_convert_block(
        sequencer_gateway_provider: SequencerGatewayProvider,
        #[case] block_n: u64,
    ) {
        let block = fetch_block_and_updates(&ChainId::Mainnet, block_n, &sequencer_gateway_provider).await.unwrap();
        let path = &format!("test-data/block_{block_n}.json");
        // serde_json::to_writer(std::fs::File::create(path).unwrap(), &block).unwrap();
        let expected = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(block, expected)
    }
}
