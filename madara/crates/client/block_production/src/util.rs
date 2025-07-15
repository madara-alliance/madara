use blockifier::{state::cached_state::StateMaps, transaction::transaction_execution::Transaction};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mp_block::header::{BlockTimestamp, GasPrices, PendingHeader};
use mp_chain_config::{L1DataAvailabilityMode, StarknetVersion};
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use starknet_api::{core::ContractAddress, StarknetApiError};
use std::{
    collections::{hash_map, HashMap, VecDeque},
    ops::{Add, AddAssign},
    sync::Arc,
    time::{Duration, SystemTime},
};

// TODO: add these to metrics
#[derive(Default, Clone, Debug)]
pub struct ExecutionStats {
    /// Number of batches executed before reaching the bouncer capacity.
    pub n_batches: usize,
    /// Number of transactions included into the block.
    pub n_added_to_block: usize,
    /// Number of transactions executed.
    pub n_executed: usize,
    /// Reverted transactions are failing transactions that are included in the block.
    pub n_reverted: usize,
    /// Rejected are txs are failing transactions that are not revertible. They are thus not included in the block
    pub n_rejected: usize,
    /// Number of declared classes.
    pub declared_classes: usize,
    /// Total L2 gas consumed by the transactions in the block.
    pub l2_gas_consumed: u64,
    /// Execution time
    pub exec_duration: Duration,
}

impl Add for ExecutionStats {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self {
            n_batches: self.n_batches + other.n_batches,
            n_added_to_block: self.n_added_to_block + other.n_added_to_block,
            n_executed: self.n_executed + other.n_executed,
            n_reverted: self.n_reverted + other.n_reverted,
            n_rejected: self.n_rejected + other.n_rejected,
            declared_classes: self.declared_classes + other.declared_classes,
            l2_gas_consumed: self.l2_gas_consumed + other.l2_gas_consumed,
            exec_duration: self.exec_duration + other.exec_duration,
        }
    }
}
impl AddAssign for ExecutionStats {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.clone() + rhs
    }
}

#[derive(Default, Debug)]
pub(crate) struct BatchToExecute {
    pub txs: Vec<Transaction>,
    pub additional_info: VecDeque<AdditionalTxInfo>,
}

impl BatchToExecute {
    pub fn with_capacity(cap: usize) -> Self {
        Self { txs: Vec::with_capacity(cap), additional_info: VecDeque::with_capacity(cap) }
    }

    pub fn extend(&mut self, other: Self) {
        self.txs.extend(other.txs);
        self.additional_info.extend(other.additional_info);
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn push(&mut self, tx: Transaction, additional_info: AdditionalTxInfo) {
        self.txs.push(tx);
        self.additional_info.push_back(additional_info);
    }

    pub fn remove_n_front(&mut self, n_to_remove: usize) -> BatchToExecute {
        // we can't actually use split_off because it doesnt leave the cap :/

        let txs = self.txs.drain(..n_to_remove).collect();
        let additional_info = self.additional_info.drain(..n_to_remove).collect();
        BatchToExecute { txs, additional_info }
    }
}

#[derive(Debug)]
pub(crate) struct AdditionalTxInfo {
    pub declared_class: Option<ConvertedClass>,
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
    pub gas_prices: GasPrices,
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
            gas_prices: self.gas_prices,
            l1_da_mode: self.l1_da_mode,
        }
    }

    pub fn to_blockifier(&self) -> Result<starknet_api::block::BlockInfo, StarknetApiError> {
        Ok(starknet_api::block::BlockInfo {
            block_number: starknet_api::block::BlockNumber(self.block_n),
            block_timestamp: starknet_api::block::BlockTimestamp(BlockTimestamp::from(self.block_timestamp).0),
            sequencer_address: self.sequencer_address.try_into()?,
            gas_prices: (&self.gas_prices).into(),
            use_kzg_da: self.l1_da_mode == L1DataAvailabilityMode::Blob,
        })
    }
}

pub(crate) fn create_execution_context(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    previous_l2_gas_price: u128,
    previous_l2_gas_used: u64,
) -> anyhow::Result<BlockExecutionContext> {
    Ok(BlockExecutionContext {
        sequencer_address: **backend.chain_config().sequencer_address,
        block_timestamp: SystemTime::now(),
        protocol_version: backend.chain_config().latest_protocol_version,
        gas_prices: backend.calculate_gas_prices(previous_l2_gas_price, previous_l2_gas_used)?,
        l1_da_mode: backend.chain_config().l1_da_mode,
        block_n,
    })
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
        nonces.insert(Felt::from_hex_unchecked("1").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("1")));
        nonces.insert(Felt::from_hex_unchecked("2").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("2")));
        nonces.insert(Felt::from_hex_unchecked("3").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("3")));

        let mut class_hashes = HashMap::new();
        class_hashes
            .insert(Felt::from_hex_unchecked("1").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a551")));
        class_hashes
            .insert(Felt::from_hex_unchecked("2").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a552")));
        class_hashes
            .insert(Felt::from_hex_unchecked("3").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a553")));

        let mut storage = HashMap::new();
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a551")),
            CompiledClassHash(Felt::from_hex_unchecked("0x1")),
        );
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a552")),
            CompiledClassHash(Felt::from_hex_unchecked("0x2")),
        );

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a551")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a552")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a553")), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("1"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("2"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("3"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![Felt::from_hex_unchecked("0xc1a553")];

        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
                compiled_class_hash: Felt::from_hex_unchecked("0x1"),
            },
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
                compiled_class_hash: Felt::from_hex_unchecked("0x2"),
            },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: Felt::from_hex_unchecked("1"), nonce: Felt::from_hex_unchecked("1") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("2"), nonce: Felt::from_hex_unchecked("2") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("3"), nonce: Felt::from_hex_unchecked("3") },
        ];

        let deployed_contracts = vec![
            DeployedContractItem {
                address: Felt::from_hex_unchecked("1"),
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("2"),
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("3"),
                class_hash: Felt::from_hex_unchecked("0xc1a553"),
            },
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
