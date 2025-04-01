use blockifier::{
    blockifier::{
        config::TransactionExecutorConfig,
        transaction_executor::{TransactionExecutor, DEFAULT_STACK_SIZE},
    },
    context::{BlockContext, ChainInfo, FeeTokenAddresses},
    state::cached_state::{CachedState, StateMaps},
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_exec::{BlockifierStateAdapter, CachedStateAdaptor};
use mc_mempool::L1DataProvider;
use mp_block::header::{BlockTimestamp, GasPrices, L1DataAvailabilityMode, PendingHeader};
use mp_chain_config::StarknetVersion;
use mp_convert::{Felt, ToFelt};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use starknet_api::{block::BlockNumber, core::ContractAddress};
use std::{
    collections::{hash_map, HashMap},
    sync::Arc,
    time::SystemTime,
};

pub(crate) fn create_blockifier_state_adaptor(
    backend: &Arc<MadaraBackend>,
    on_block_n: Option<u64>,
) -> CachedStateAdaptor {
    let block_n = on_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
    CachedStateAdaptor::new(BlockifierStateAdapter::new(
        Arc::clone(backend),
        block_n,
        on_block_n.map(DbBlockId::Number),
    ))
}

/// This is a pending header, without parent_block_hash. Parent block hash is not visible to the execution,
/// and in addition, we can't know it yet without closing the block and updating the global trie to compute
/// the global state root.
/// See [`crate::executor::Executor`]; we want to be able to start the execution of new blocks without waiting
/// on the earlier to be closed.
#[derive(Debug, Clone)]
pub(crate) struct BlockExecutionContext {
    /// The new block_n.
    pub block_n: u64,
    /// The Starknet address of the sequencer who created this block.
    pub sequencer_address: Felt,
    /// Unix timestamp (seconds) when the block was produced -- before executing any transaction.
    pub block_timestamp: SystemTime, // We use a systemtime here for better logging.
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

impl BlockExecutionContext {
    pub fn into_header(self, parent_block_hash: Felt) -> PendingHeader {
        PendingHeader {
            parent_block_hash,
            sequencer_address: self.sequencer_address,
            block_timestamp: self.block_timestamp.into(),
            protocol_version: self.protocol_version,
            l1_gas_price: self.l1_gas_price,
            l1_da_mode: self.l1_da_mode,
        }
    }
}

pub(crate) fn create_execution_context(
    l1_data_provider: &Arc<dyn L1DataProvider>,
    backend: &Arc<MadaraBackend>,
    on_block_n: Option<u64>,
) -> BlockExecutionContext {
    let chain_config = backend.chain_config();
    let block_n = on_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
    BlockExecutionContext {
        sequencer_address: **chain_config.sequencer_address,
        block_timestamp: SystemTime::now(),
        protocol_version: chain_config.latest_protocol_version,
        l1_gas_price: l1_data_provider.get_gas_prices(),
        l1_da_mode: l1_data_provider.get_da_mode(),
        block_n,
    }
}

pub(crate) fn create_blockifier_executor(
    adaptor: CachedStateAdaptor,
    backend: &Arc<MadaraBackend>,
    ctx: &BlockExecutionContext,
) -> anyhow::Result<TransactionExecutor<CachedStateAdaptor>> {
    let chain_config = backend.chain_config();

    // We dont care about the parent block hash, it is not used during execution.
    let concurrency_config = chain_config.concurrency_config.clone();

    let versioned_constants = chain_config.exec_constants_by_protocol_version(ctx.protocol_version)?;
    let chain_info = ChainInfo {
        chain_id: chain_config.chain_id.clone(),
        fee_token_addresses: FeeTokenAddresses {
            strk_fee_token_address: chain_config.native_fee_token_address,
            eth_fee_token_address: chain_config.parent_fee_token_address,
        },
    };
    let block_info = starknet_api::block::BlockInfo {
        block_timestamp: starknet_api::block::BlockTimestamp(BlockTimestamp::from(ctx.block_timestamp).0),
        block_number: BlockNumber(ctx.block_n),
        sequencer_address: chain_config.sequencer_address,
        gas_prices: (&ctx.l1_gas_price).into(),
        use_kzg_da: ctx.l1_da_mode == L1DataAvailabilityMode::Blob,
    };
    let blockifier_executor = TransactionExecutor::new(
        CachedState::new(adaptor),
        BlockContext::new(block_info, chain_info, versioned_constants, chain_config.bouncer_config.clone()),
        TransactionExecutorConfig { concurrency_config, stack_size: DEFAULT_STACK_SIZE },
    );
    Ok(blockifier_executor)
}

pub(crate) fn state_map_to_state_diff(
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
    diff: StateMaps,
) -> anyhow::Result<StateDiff> {
    let mut backing_map = HashMap::<ContractAddress, usize>::default();
    let mut storage_diffs = Vec::<ContractStorageDiffItem>::default();
    for ((address, key), value) in diff.storage {
        match backing_map.entry(address) {
            hash_map::Entry::Vacant(e) => {
                e.insert(storage_diffs.len());
                storage_diffs.push(ContractStorageDiffItem {
                    address: address.to_felt(),
                    storage_entries: vec![StorageEntry { key: key.to_felt(), value }],
                });
            }
            hash_map::Entry::Occupied(e) => {
                storage_diffs[*e.get()].storage_entries.push(StorageEntry { key: key.to_felt(), value });
            }
        }
    }

    let mut deprecated_declared_classes = Vec::default();
    for (class_hash, _) in diff.declared_contracts {
        if !diff.compiled_class_hashes.contains_key(&class_hash) {
            deprecated_declared_classes.push(class_hash.to_felt());
        }
    }

    let declared_classes = diff
        .compiled_class_hashes
        .iter()
        .map(|(class_hash, compiled_class_hash)| DeclaredClassItem {
            class_hash: class_hash.to_felt(),
            compiled_class_hash: compiled_class_hash.to_felt(),
        })
        .collect();

    let nonces = diff
        .nonces
        .into_iter()
        .map(|(contract_address, nonce)| NonceUpdate {
            contract_address: contract_address.to_felt(),
            nonce: nonce.to_felt(),
        })
        .collect();

    let mut deployed_contracts = Vec::new();
    let mut replaced_classes = Vec::new();
    for (contract_address, new_class_hash) in diff.class_hashes {
        let replaced = if let Some(on_top_of) = on_top_of {
            match backend.get_contract_class_hash_at(on_top_of, &contract_address.to_felt())? {
                Some(class_hash) => class_hash != new_class_hash.to_felt(),
                None => false,
            }
        } else {
            // Executing genesis block: nothing being redefined here
            false
        };
        if replaced {
            replaced_classes.push(ReplacedClassItem {
                contract_address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        } else {
            deployed_contracts.push(DeployedContractItem {
                address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        }
    }

    Ok(StateDiff {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        nonces,
        deployed_contracts,
        replaced_classes,
    })
}

#[cfg(test)]
mod test {
    use blockifier::state::cached_state::StateMaps;
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_convert::felt;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry,
    };
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
    use starknet_core::types::Felt;
    use std::{collections::HashMap, sync::Arc};

    #[test]
    fn state_map_to_state_diff() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let mut nonces = HashMap::new();
        nonces.insert(felt!("1").try_into().unwrap(), Nonce(felt!("1")));
        nonces.insert(felt!("2").try_into().unwrap(), Nonce(felt!("2")));
        nonces.insert(felt!("3").try_into().unwrap(), Nonce(felt!("3")));

        let mut class_hashes = HashMap::new();
        class_hashes.insert(felt!("1").try_into().unwrap(), ClassHash(felt!("0xc1a551")));
        class_hashes.insert(felt!("2").try_into().unwrap(), ClassHash(felt!("0xc1a552")));
        class_hashes.insert(felt!("3").try_into().unwrap(), ClassHash(felt!("0xc1a553")));

        let mut storage = HashMap::new();
        storage.insert((felt!("1").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("1").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("1").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        storage.insert((felt!("2").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("2").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("2").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        storage.insert((felt!("3").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("3").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("3").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(ClassHash(felt!("0xc1a551")), CompiledClassHash(felt!("0x1")));
        compiled_class_hashes.insert(ClassHash(felt!("0xc1a552")), CompiledClassHash(felt!("0x2")));

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(ClassHash(felt!("0xc1a551")), true);
        declared_contracts.insert(ClassHash(felt!("0xc1a552")), true);
        declared_contracts.insert(ClassHash(felt!("0xc1a553")), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: felt!("1"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!("2"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!("3"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![felt!("0xc1a553")];

        let declared_classes = vec![
            DeclaredClassItem { class_hash: felt!("0xc1a551"), compiled_class_hash: felt!("0x1") },
            DeclaredClassItem { class_hash: felt!("0xc1a552"), compiled_class_hash: felt!("0x2") },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: felt!("1"), nonce: felt!("1") },
            NonceUpdate { contract_address: felt!("2"), nonce: felt!("2") },
            NonceUpdate { contract_address: felt!("3"), nonce: felt!("3") },
        ];

        let deployed_contracts = vec![
            DeployedContractItem { address: felt!("1"), class_hash: felt!("0xc1a551") },
            DeployedContractItem { address: felt!("2"), class_hash: felt!("0xc1a552") },
            DeployedContractItem { address: felt!("3"), class_hash: felt!("0xc1a553") },
        ];

        let replaced_classes = vec![];

        let expected = StateDiff {
            storage_diffs,
            deprecated_declared_classes,
            declared_classes,
            nonces,
            deployed_contracts,
            replaced_classes,
        };

        let mut actual = super::state_map_to_state_diff(&backend, &Option::<_>::None, state_map).unwrap();

        actual.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
        actual.storage_diffs.iter_mut().for_each(|s| s.storage_entries.sort_by(|a, b| a.key.cmp(&b.key)));
        actual.deprecated_declared_classes.sort();
        actual.declared_classes.sort_by(|a, b| a.class_hash.cmp(&b.class_hash));
        actual.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
        actual.deployed_contracts.sort_by(|a, b| a.address.cmp(&b.address));
        actual.replaced_classes.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));

        assert_eq!(
            actual,
            expected,
            "actual: {}\nexpected: {}",
            serde_json::to_string_pretty(&actual).unwrap_or_default(),
            serde_json::to_string_pretty(&expected).unwrap_or_default()
        );
    }
}
