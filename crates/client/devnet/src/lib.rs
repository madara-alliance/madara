use anyhow::Context;
use blockifier::abi::abi_utils::get_storage_var_address;
use mc_block_import::{UnverifiedFullBlock, UnverifiedHeader};
use mp_block::header::GasPrices;
use mp_chain_config::ChainConfig;
use mp_convert::ToFelt;
use mp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use starknet_api::{core::ContractAddress, state::StorageKey};
use starknet_signers::SigningKey;
use starknet_types_core::felt::Felt;
use std::{collections::HashMap, time::SystemTime};

mod balances;
mod classes;
mod contracts;
mod entrypoint;
mod predeployed_contracts;

pub use balances::*;
pub use classes::*;
pub use contracts::*;
pub use entrypoint::*;
use mp_transactions::compute_hash::calculate_contract_address;
pub use predeployed_contracts::*;

// 1 ETH = 1e18 WEI
const ETH_WEI_DECIMALS: u128 = 1_000_000_000_000_000_000;
// 1 STRK = 1e18 FRI
const STRK_FRI_DECIMALS: u128 = 1_000_000_000_000_000_000;

#[derive(Debug, Clone, Default)]
pub struct StorageDiffs(HashMap<ContractAddress, HashMap<StorageKey, Felt>>);
impl StorageDiffs {
    pub fn contract_mut(&mut self, contract_address: ContractAddress) -> &mut HashMap<StorageKey, Felt> {
        self.0.entry(contract_address).or_default()
    }

    pub fn as_state_diff(&self) -> Vec<ContractStorageDiffItem> {
        self.0
            .iter()
            .map(|(contract, map)| ContractStorageDiffItem {
                address: contract.to_felt(),
                storage_entries: map.iter().map(|(key, &value)| StorageEntry { key: key.to_felt(), value }).collect(),
            })
            .collect()
    }
}

// We allow ourselves to lie about the contract_address. This is because we want the UDC and the two ERC20 contracts to have well known addresses on every chain.

/// Universal Deployer Contract.
const UDC_CLASS_DEFINITION: &[u8] = include_bytes!("../../../../cairo_0/madara_contracts_UDC.json");
const UDC_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

const ERC20_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../cairo/target/dev/madara_contracts_ERC20.contract_class.json");
const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../cairo/target/dev/madara_contracts_AccountUpgradeable.contract_class.json");

/// High level description of the genesis block.
#[derive(Clone, Debug, Default)]
pub struct ChainGenesisDescription {
    pub initial_balances: InitialBalances,
    pub declared_classes: InitiallyDeclaredClasses,
    pub deployed_contracts: InitiallyDeployedContracts,
    /// This is filled in with the initial_balances too when building.
    pub initial_storage: StorageDiffs,
}

impl ChainGenesisDescription {
    pub fn base_config() -> anyhow::Result<Self> {
        let udc_class = InitiallyDeclaredClass::new_legacy(UDC_CLASS_DEFINITION).context("Failed to add UDC class")?;
        let erc20_class =
            InitiallyDeclaredClass::new_sierra(ERC20_CLASS_DEFINITION).context("Failed to add ERC20 class")?;
        Ok(Self {
            initial_balances: InitialBalances::default(),
            deployed_contracts: InitiallyDeployedContracts::default()
                .with(UDC_CONTRACT_ADDRESS, udc_class.class_hash())
                .with(ERC20_ETH_CONTRACT_ADDRESS, erc20_class.class_hash())
                .with(ERC20_STRK_CONTRACT_ADDRESS, erc20_class.class_hash()),
            declared_classes: InitiallyDeclaredClasses::default().with(udc_class).with(erc20_class),
            initial_storage: StorageDiffs::default(),
        })
    }

    pub fn add_devnet_contracts(&mut self, n_addr: u64) -> anyhow::Result<DevnetKeys> {
        let account_class =
            InitiallyDeclaredClass::new_sierra(ACCOUNT_CLASS_DEFINITION).context("Failed to add account class")?;
        let account_class_hash = account_class.class_hash();
        self.declared_classes.insert(account_class);

        fn get_contract_pubkey_storage_address() -> StorageKey {
            get_storage_var_address("Account_public_key", &[])
        }

        pub fn from_seed(seed: u64) -> Felt {
            // Use a fixed seed for deterministic RNG
            let mut rng = StdRng::seed_from_u64(seed);
            let mut buffer = [0u8; 32];
            rng.fill_bytes(&mut buffer);

            Felt::from_bytes_be_slice(&buffer)
        }

        Ok(DevnetKeys(
            (0..n_addr)
                .map(|addr_idx| {
                    let secret_scalar = from_seed(addr_idx);
                    let key = SigningKey::from_secret_scalar(secret_scalar);
                    let pubkey = key.verifying_key();

                    // calculating actual address w.r.t. the class hash.
                    let calculated_address =
                        calculate_contract_address(Felt::ZERO, account_class_hash, &[pubkey.scalar()], Felt::ZERO);

                    let balance = ContractFeeTokensBalance {
                        fri: (10_000 * ETH_WEI_DECIMALS).into(),
                        wei: (10_000 * STRK_FRI_DECIMALS).into(),
                    };

                    self.deployed_contracts.insert(calculated_address, account_class_hash);
                    self.initial_balances
                        .insert(ContractAddress::try_from(calculated_address).unwrap(), balance.clone());
                    self.initial_storage
                        .contract_mut(calculated_address.try_into().unwrap())
                        .insert(get_contract_pubkey_storage_address(), pubkey.scalar());

                    DevnetPredeployedContract {
                        secret: key,
                        pubkey: pubkey.scalar(),
                        balance,
                        address: calculated_address,
                        class_hash: account_class_hash,
                    }
                })
                .collect(),
        ))
    }

    pub fn build(mut self, chain_config: &ChainConfig) -> anyhow::Result<UnverifiedFullBlock> {
        self.initial_balances.to_storage_diffs(chain_config, &mut self.initial_storage);

        Ok(UnverifiedFullBlock {
            header: UnverifiedHeader {
                parent_block_hash: Some(Felt::ZERO),
                sequencer_address: chain_config.sequencer_address.to_felt(),
                block_timestamp: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("Current time is before unix epoch!")
                    .as_secs(),
                protocol_version: chain_config.latest_protocol_version,
                l1_gas_price: GasPrices {
                    eth_l1_gas_price: 5,
                    strk_l1_gas_price: 5,
                    eth_l1_data_gas_price: 5,
                    strk_l1_data_gas_price: 5,
                },
                l1_da_mode: mp_block::header::L1DataAvailabilityMode::Blob,
            },
            state_diff: StateDiff {
                storage_diffs: self.initial_storage.as_state_diff(),
                deprecated_declared_classes: self.declared_classes.as_legacy_state_diff(),
                declared_classes: self.declared_classes.as_state_diff(),
                deployed_contracts: self.deployed_contracts.as_state_diff(),
                replaced_classes: vec![],
                nonces: vec![],
            },
            declared_classes: self.declared_classes.into_loaded_classes(),
            unverified_block_number: Some(0),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use mc_block_import::{BlockImporter, BlockValidationContext};
    use mc_db::MadaraBackend;
    use mc_mempool::block_production::BlockProductionTask;
    use mc_mempool::MempoolProvider;
    use mc_mempool::{transaction_hash, L1DataProvider, Mempool, MockL1DataProvider};
    use mc_metrics::MetricsRegistry;
    use mp_block::header::L1DataAvailabilityMode;
    use mp_block::{BlockId, BlockTag};
    use mp_class::ClassInfo;
    use mp_convert::felt_to_u128;
    use mp_receipt::{Event, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt};
    use mp_transactions::broadcasted_to_blockifier;
    use mp_transactions::compute_hash::calculate_contract_address;
    use mp_utils::tests_common::*;
    use rstest::{fixture, rstest};
    use starknet_core::types::contract::SierraClass;
    use starknet_core::types::{
        BroadcastedDeclareTransaction, BroadcastedDeclareTransactionV3, BroadcastedDeployAccountTransaction,
        BroadcastedDeployAccountTransactionV3, BroadcastedInvokeTransaction, BroadcastedInvokeTransactionV3,
        BroadcastedTransaction, DataAvailabilityMode, DeclareTransactionResult, DeployAccountTransactionResult,
        FlattenedSierraClass, InvokeTransactionResult, ResourceBounds, ResourceBoundsMapping,
    };
    use std::sync::Arc;

    struct DevnetForTesting {
        backend: Arc<MadaraBackend>,
        contracts: DevnetKeys,
        block_production: BlockProductionTask<Mempool>,
        mempool: Arc<Mempool>,
    }

    impl DevnetForTesting {
        pub fn sign_and_add_invoke_tx(
            &self,
            mut tx: BroadcastedInvokeTransaction,
            contract: &DevnetPredeployedContract,
        ) -> InvokeTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::Invoke(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
                self.backend.chain_config().latest_protocol_version,
            )
            .unwrap();
            let signature = contract.secret.sign(&transaction_hash(&blockifier_tx)).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedInvokeTransaction::V1(tx) => &mut tx.signature,
                BroadcastedInvokeTransaction::V3(tx) => &mut tx.signature,
            };
            *tx_signature = vec![signature.r, signature.s];

            log::debug!("tx: {:?}", tx);

            self.mempool.accept_invoke_tx(tx).unwrap()
        }

        pub fn sign_and_add_declare_tx(
            &self,
            mut tx: BroadcastedDeclareTransaction,
            contract: &DevnetPredeployedContract,
        ) -> DeclareTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::Declare(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
                self.backend.chain_config().latest_protocol_version,
            )
            .unwrap();
            let signature = contract.secret.sign(&transaction_hash(&blockifier_tx)).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedDeclareTransaction::V1(tx) => &mut tx.signature,
                BroadcastedDeclareTransaction::V2(tx) => &mut tx.signature,
                BroadcastedDeclareTransaction::V3(tx) => &mut tx.signature,
            };
            *tx_signature = vec![signature.r, signature.s];

            self.mempool.accept_declare_tx(tx).unwrap()
        }

        pub fn sign_and_add_deploy_account_tx(
            &self,
            mut tx: BroadcastedDeployAccountTransaction,
            contract: &DevnetPredeployedContract,
        ) -> DeployAccountTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::DeployAccount(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
                self.backend.chain_config().latest_protocol_version,
            )
            .unwrap();
            let signature = contract.secret.sign(&transaction_hash(&blockifier_tx)).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedDeployAccountTransaction::V1(tx) => &mut tx.signature,
                BroadcastedDeployAccountTransaction::V3(tx) => &mut tx.signature,
            };
            *tx_signature = vec![signature.r, signature.s];

            self.mempool.accept_deploy_account_tx(tx).unwrap()
        }

        /// (STRK in FRI, ETH in WEI)
        pub fn get_bal_strk_eth(&self, contract_address: Felt) -> (u128, u128) {
            get_fee_tokens_balance(&self.backend, contract_address).unwrap().as_u128_fri_wei().unwrap()
        }
    }

    #[fixture]
    fn chain() -> DevnetForTesting {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut g = ChainGenesisDescription::base_config().unwrap();
        let contracts = g.add_devnet_contracts(10).unwrap();

        let chain_config = Arc::new(ChainConfig::test_config().unwrap());
        let block = g.build(&chain_config).unwrap();
        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        let importer =
            Arc::new(BlockImporter::new(Arc::clone(&backend), &MetricsRegistry::dummy(), None, true).unwrap());

        println!("{:?}", block.state_diff);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(
                importer.add_block(
                    block,
                    BlockValidationContext::new(chain_config.chain_id.clone()).trust_class_hashes(true),
                ),
            )
            .unwrap();

        log::debug!("{:?}", backend.get_block_info(&BlockId::Tag(BlockTag::Latest)));

        let mut l1_data_provider = MockL1DataProvider::new();
        l1_data_provider.expect_get_da_mode().return_const(L1DataAvailabilityMode::Blob);
        l1_data_provider.expect_get_gas_prices().return_const(GasPrices {
            eth_l1_gas_price: 128,
            strk_l1_gas_price: 128,
            eth_l1_data_gas_price: 128,
            strk_l1_data_gas_price: 128,
        });
        let l1_data_provider = Arc::new(l1_data_provider) as Arc<dyn L1DataProvider>;
        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), Arc::clone(&l1_data_provider)));
        let block_production = BlockProductionTask::new(
            Arc::clone(&backend),
            Arc::clone(&importer),
            Arc::clone(&mempool),
            Arc::clone(&l1_data_provider),
        )
        .unwrap();

        DevnetForTesting { backend, contracts, block_production, mempool }
    }

    #[rstest]
    #[case("./cairo/target/dev/madara_contracts_TestContract.contract_class.json")]
    fn test_erc_20_declare(_set_workdir: (), mut chain: DevnetForTesting, #[case] contract_path: &str) {
        println!("{}", chain.contracts);

        let sender_address = &chain.contracts.0[0];

        let sierra_class: SierraClass = serde_json::from_reader(std::fs::File::open(contract_path).unwrap()).unwrap();
        let flattened_class: FlattenedSierraClass = sierra_class.clone().flatten().unwrap();

        // starkli class-hash target/dev/madara_contracts_TestContract.compiled_contract_class.json
        let compiled_contract_class_hash =
            Felt::from_hex("0x0138105ded3d2e4ea1939a0bc106fb80fd8774c9eb89c1890d4aeac88e6a1b27").unwrap();

        let declare_txn: BroadcastedDeclareTransaction =
            BroadcastedDeclareTransaction::V3(BroadcastedDeclareTransactionV3 {
                sender_address: sender_address.address,
                compiled_class_hash: compiled_contract_class_hash,
                signature: vec![],
                nonce: Felt::ZERO,
                contract_class: Arc::new(flattened_class),
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 210000, max_price_per_unit: 10000 },
                    l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                },
                tip: 0,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L1,
                is_query: false,
            });

        let res = chain.sign_and_add_declare_tx(declare_txn, sender_address);

        let calculated_class_hash = sierra_class.class_hash().unwrap();

        assert_eq!(res.class_hash, calculated_class_hash);

        chain.block_production.set_current_pending_tick(1);
        chain.block_production.on_pending_time_tick().unwrap();

        let block = chain.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap();

        assert_eq!(block.inner.transactions.len(), 1);
        assert_eq!(block.inner.receipts.len(), 1);
        log::debug!("receipt: {:?}", block.inner.receipts[0]);

        let class_info =
            chain.backend.get_class_info(&BlockId::Tag(BlockTag::Pending), &calculated_class_hash).unwrap().unwrap();

        assert_matches!(
            class_info,
            ClassInfo::Sierra(info) if info.compiled_class_hash == compiled_contract_class_hash
        );

        let TransactionReceipt::Declare(receipt) = block.inner.receipts[0].clone() else { unreachable!() };

        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);
    }

    #[rstest]
    fn test_account_deploy(_set_workdir: (), mut chain: DevnetForTesting) {
        let key = SigningKey::from_random();
        log::debug!("Secret Key : {:?}", key.secret_scalar());

        let pubkey = key.verifying_key();
        log::debug!("Public Key : {:?}", pubkey.scalar());

        // using the class hash of the first account as the account class hash
        let account_class_hash = chain.contracts.0[0].class_hash;
        let calculated_address =
            calculate_contract_address(Felt::ZERO, account_class_hash, &[pubkey.scalar()], Felt::ZERO);
        log::debug!("Calculated Address : {:?}", calculated_address);

        // =====================================================================================
        // Transferring the funds from pre deployed account into the calculated address
        let contract_0 = &chain.contracts.0[0];

        let transfer_txn = chain.sign_and_add_invoke_tx(
            BroadcastedInvokeTransaction::V3(BroadcastedInvokeTransactionV3 {
                sender_address: contract_0.address,
                calldata: Multicall::default()
                    .with(Call {
                        to: ERC20_STRK_CONTRACT_ADDRESS,
                        selector: Selector::from("transfer"),
                        calldata: vec![calculated_address, (9_999u128 * STRK_FRI_DECIMALS).into(), Felt::ZERO],
                    })
                    .flatten()
                    .collect(),
                signature: vec![], // Signature is filled in by `sign_and_add_invoke_tx`.
                nonce: Felt::ZERO,
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                    l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                },
                tip: 0,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: starknet_core::types::DataAvailabilityMode::L1,
                fee_data_availability_mode: starknet_core::types::DataAvailabilityMode::L1,
                is_query: false,
            }),
            contract_0,
        );
        log::debug!("tx hash: {:#x}", transfer_txn.transaction_hash);

        chain.block_production.set_current_pending_tick(chain.backend.chain_config().n_pending_ticks_per_block());
        chain.block_production.on_pending_time_tick().unwrap();

        // =====================================================================================

        let account_balance = get_fee_tokens_balance(&chain.backend, calculated_address).unwrap();
        let account = DevnetPredeployedContract {
            secret: key,
            pubkey: pubkey.scalar(),
            balance: account_balance,
            address: calculated_address,
            class_hash: account_class_hash,
        };

        let deploy_account_txn = BroadcastedDeployAccountTransaction::V3(BroadcastedDeployAccountTransactionV3 {
            signature: vec![],
            nonce: Felt::ZERO,
            contract_address_salt: Felt::ZERO,
            constructor_calldata: vec![pubkey.scalar()],
            class_hash: account_class_hash,
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            },
            tip: 0,
            paymaster_data: vec![],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
            is_query: false,
        });

        let res = chain.sign_and_add_deploy_account_tx(deploy_account_txn, &account);

        chain.block_production.set_current_pending_tick(chain.backend.chain_config().n_pending_ticks_per_block());
        chain.block_production.on_pending_time_tick().unwrap();

        assert_eq!(res.contract_address, account.address);

        let block = chain.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap();

        assert_eq!(block.inner.transactions.len(), 2);
        assert_eq!(block.inner.receipts.len(), 2);

        let TransactionReceipt::DeployAccount(receipt) = block.inner.receipts[1].clone() else { unreachable!() };

        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);
    }

    // TODO: add eth transfer
    #[rstest]
    #[case(24235u128, false)]
    #[case(9_999u128 * STRK_FRI_DECIMALS, false)]
    #[case(10_001u128 * STRK_FRI_DECIMALS, true)]
    fn test_basic_transfer(
        _set_workdir: (),
        mut chain: DevnetForTesting,
        #[case] transfer_amount: u128,
        #[case] expect_reverted: bool,
    ) {
        println!("{}", chain.contracts);

        let sequencer_address = chain.backend.chain_config().sequencer_address.to_felt();
        let contract_0 = &chain.contracts.0[0];
        let contract_1 = &chain.contracts.0[1];

        assert_eq!(chain.get_bal_strk_eth(sequencer_address), (0, 0));
        assert_eq!(chain.get_bal_strk_eth(contract_0.address), (10_000 * STRK_FRI_DECIMALS, 10_000 * ETH_WEI_DECIMALS));
        assert_eq!(chain.get_bal_strk_eth(contract_1.address), (10_000 * STRK_FRI_DECIMALS, 10_000 * ETH_WEI_DECIMALS));

        let result = chain.sign_and_add_invoke_tx(
            BroadcastedInvokeTransaction::V3(BroadcastedInvokeTransactionV3 {
                sender_address: contract_0.address,
                calldata: Multicall::default()
                    .with(Call {
                        to: ERC20_STRK_CONTRACT_ADDRESS,
                        selector: Selector::from("transfer"),
                        calldata: vec![contract_1.address, transfer_amount.into(), Felt::ZERO],
                    })
                    .flatten()
                    .collect(),
                signature: vec![], // Signature is filled in by `sign_and_add_invoke_tx`.
                nonce: Felt::ZERO,
                resource_bounds: ResourceBoundsMapping {
                    l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                    l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                },
                tip: 0,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: starknet_core::types::DataAvailabilityMode::L1,
                fee_data_availability_mode: starknet_core::types::DataAvailabilityMode::L1,
                is_query: false,
            }),
            contract_0,
        );

        log::info!("tx hash: {:#x}", result.transaction_hash);

        chain.block_production.set_current_pending_tick(1);
        chain.block_production.on_pending_time_tick().unwrap();

        let block = chain.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap();

        assert_eq!(block.inner.transactions.len(), 1);
        assert_eq!(block.inner.receipts.len(), 1);
        log::info!("receipt: {:?}", block.inner.receipts[0]);

        let TransactionReceipt::Invoke(receipt) = block.inner.receipts[0].clone() else { unreachable!() };

        if !expect_reverted {
            assert_eq!(
                receipt,
                InvokeTransactionReceipt {
                    transaction_hash: result.transaction_hash,
                    messages_sent: vec![],
                    events: vec![Event {
                        from_address: ERC20_STRK_CONTRACT_ADDRESS,
                        // TODO: do not match keys and data yet (unsure)
                        keys: receipt.events[0].keys.clone(),
                        data: receipt.events[0].data.clone(),
                    }],
                    // TODO: resources and fees are not tested because they consistent accross runs, we have to figure out why
                    execution_resources: receipt.execution_resources.clone(),
                    actual_fee: FeePayment { amount: receipt.actual_fee.amount, unit: PriceUnit::Fri },
                    execution_result: receipt.execution_result.clone(), // matched below
                }
            );
        }

        match expect_reverted {
            false => {
                assert_eq!(&receipt.execution_result, &ExecutionResult::Succeeded);

                let fees_fri = felt_to_u128(&block.inner.receipts[0].actual_fee().amount).unwrap();
                assert_eq!(chain.get_bal_strk_eth(sequencer_address), (fees_fri, 0));
                assert_eq!(
                    chain.get_bal_strk_eth(contract_0.address),
                    (10_000 * STRK_FRI_DECIMALS - fees_fri - transfer_amount, 10_000 * ETH_WEI_DECIMALS)
                );
                assert_eq!(
                    chain.get_bal_strk_eth(contract_1.address),
                    (10_000 * STRK_FRI_DECIMALS + transfer_amount, 10_000 * ETH_WEI_DECIMALS)
                );
            }
            true => {
                let ExecutionResult::Reverted { reason } = receipt.execution_result else { unreachable!() };
                assert!(reason.contains("ERC20: insufficient balance"));

                let fees_fri = felt_to_u128(&block.inner.receipts[0].actual_fee().amount).unwrap();
                assert_eq!(chain.get_bal_strk_eth(sequencer_address), (fees_fri, 0));
                assert_eq!(
                    chain.get_bal_strk_eth(contract_0.address),
                    (10_000 * STRK_FRI_DECIMALS - fees_fri, 10_000 * ETH_WEI_DECIMALS)
                );
                assert_eq!(
                    chain.get_bal_strk_eth(contract_1.address),
                    (10_000 * STRK_FRI_DECIMALS, 10_000 * ETH_WEI_DECIMALS)
                );
            }
        }
    }
}
