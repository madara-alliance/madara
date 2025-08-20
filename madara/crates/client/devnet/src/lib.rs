use anyhow::Context;
use mc_db::MadaraBackend;
use mp_block::{
    header::{GasPrices, PreconfirmedHeader},
    PreconfirmedFullBlock,
};
use mp_chain_config::ChainConfig;
use mp_class::ClassInfoWithHash;
use mp_convert::ToFelt;
use mp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
use starknet_api::abi::abi_utils::get_storage_var_address;
use starknet_api::{core::ContractAddress, state::StorageKey};
use starknet_signers::SigningKey;
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};
use std::{collections::HashMap, sync::Arc, time::SystemTime};

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
const UDC_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../build-artifacts/cairo_artifacts/madara_contracts_UDC.json");
pub const UDC_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

const ERC20_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../build-artifacts/cairo_artifacts/openzeppelin_ERC20Upgradeable.contract_class.json");
const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_CLASS_DEFINITION: &[u8] = include_bytes!(
    "../../../../../build-artifacts/cairo_artifacts/openzeppelin_AccountUpgradeable.contract_class.json"
);

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
    #[tracing::instrument(fields(module = "ChainGenesisDescription"))]
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

    #[tracing::instrument(skip(self), fields(module = "ChainGenesisDescription"))]
    pub fn add_devnet_contracts(&mut self, n_addr: u64) -> anyhow::Result<DevnetKeys> {
        let account_class =
            InitiallyDeclaredClass::new_sierra(ACCOUNT_CLASS_DEFINITION).context("Failed to add account class")?;
        let account_class_hash = account_class.class_hash();
        self.declared_classes.insert(account_class);

        fn get_contract_pubkey_storage_address() -> StorageKey {
            get_storage_var_address("Account_public_key", &[])
        }

        // We may want to make this seed a cli argument in the future.
        let seed = Felt::from_hex_unchecked("0x1278b36872363a1276387");

        fn rand_from_i(seed: Felt, i: u64) -> Felt {
            Poseidon::hash(&seed, &(31 ^ !i).into())
        }

        Ok(DevnetKeys(
            (0..n_addr)
                .map(|addr_idx| {
                    let secret_scalar = rand_from_i(seed, addr_idx);
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

    #[tracing::instrument(skip(self, chain_config), fields(module = "ChainGenesisDescription"))]
    pub fn into_block(
        mut self,
        chain_config: &ChainConfig,
    ) -> anyhow::Result<(PreconfirmedFullBlock, Vec<ClassInfoWithHash>)> {
        self.initial_balances.to_storage_diffs(chain_config, &mut self.initial_storage);

        Ok((
            PreconfirmedFullBlock {
                header: PreconfirmedHeader {
                    block_number: 0,
                    parent_block_hash: Felt::ZERO,
                    sequencer_address: chain_config.sequencer_address.to_felt(),
                    block_timestamp: mp_block::header::BlockTimestamp(
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .expect("Current time is before unix epoch!")
                            .as_secs(),
                    ),
                    protocol_version: chain_config.latest_protocol_version,
                    l1_gas_price: GasPrices {
                        eth_l1_gas_price: 5,
                        strk_l1_gas_price: 5,
                        eth_l1_data_gas_price: 5,
                        strk_l1_data_gas_price: 5,
                    },
                    l1_da_mode: chain_config.l1_da_mode,
                },
                state_diff: StateDiff {
                    storage_diffs: self.initial_storage.as_state_diff(),
                    deprecated_declared_classes: self.declared_classes.as_legacy_state_diff(),
                    declared_classes: self.declared_classes.as_state_diff(),
                    deployed_contracts: self.deployed_contracts.as_state_diff(),
                    replaced_classes: vec![],
                    nonces: vec![],
                },
                transactions: vec![],
                events: vec![],
            },
            self.declared_classes.into_class_infos(),
        ))
    }

    pub async fn build_and_store(self, backend: &Arc<MadaraBackend>) -> anyhow::Result<()> {
        let (block, classes) = self.into_block(backend.chain_config()).unwrap();

        let classes: Vec<_> = classes.into_iter().map(|class| class.convert()).collect::<Result<_, _>>()?;

        backend
            .write_access()
            .add_full_block_with_classes(&block, &classes, /* pre_v0_13_2_hash_override */ true)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use mc_block_production::metrics::BlockProductionMetrics;
    use mc_block_production::{BlockProductionStateNotification, BlockProductionTask};
    use mc_db::MadaraBackend;
    use mc_mempool::{L1DataProvider, Mempool, MempoolConfig, MockL1DataProvider};
    use mc_submit_tx::{
        RejectedTransactionError, RejectedTransactionErrorKind, SubmitTransaction, SubmitTransactionError,
        TransactionValidator, TransactionValidatorConfig,
    };
    use mp_class::{ClassInfo, FlattenedSierraClass};
    use mp_receipt::{Event, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt};
    use mp_rpc::{
        AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedDeployAccountTxn,
        BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, ContractAndTxnHash, DaMode, DeployAccountTxnV3,
        InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
    };
    use mp_transactions::compute_hash::calculate_contract_address;
    use mp_transactions::validated::TxTimestamp;
    use mp_transactions::IntoStarknetApiExt;
    use mp_utils::service::ServiceContext;
    use mp_utils::AbortOnDrop;
    use rstest::rstest;
    use starknet_core::types::contract::SierraClass;
    use std::sync::Arc;
    use std::time::Duration;

    struct DevnetForTesting {
        backend: Arc<MadaraBackend>,
        contracts: DevnetKeys,
        block_production: Option<BlockProductionTask>,
        tx_validator: Arc<TransactionValidator>,
    }

    impl DevnetForTesting {
        pub async fn sign_and_add_invoke_tx(
            &self,
            mut tx: BroadcastedInvokeTxn,
            contract: &DevnetPredeployedContract,
        ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
            let api_tx = BroadcastedTxn::Invoke(tx.clone())
                .into_validated_tx(
                    self.backend.chain_config().chain_id.to_felt(),
                    self.backend.chain_config().latest_protocol_version,
                    TxTimestamp::now(),
                )
                .unwrap();
            let signature = contract.secret.sign(&api_tx.tx_hash).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedInvokeTxn::V0(tx) => &mut tx.signature,
                BroadcastedInvokeTxn::V1(tx) => &mut tx.signature,
                BroadcastedInvokeTxn::V3(tx) => &mut tx.signature,
                _ => unreachable!("the invoke tx is not query only"),
            };
            *tx_signature = vec![signature.r, signature.s].into();

            tracing::debug!("tx: {:?}", tx);

            self.tx_validator.submit_invoke_transaction(tx).await
        }

        pub async fn sign_and_add_declare_tx(
            &self,
            mut tx: BroadcastedDeclareTxn,
            contract: &DevnetPredeployedContract,
        ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
            let api_tx = BroadcastedTxn::Declare(tx.clone())
                .into_validated_tx(
                    self.backend.chain_config().chain_id.to_felt(),
                    self.backend.chain_config().latest_protocol_version,
                    TxTimestamp::now(),
                )
                .unwrap();
            let signature = contract.secret.sign(&api_tx.tx_hash).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedDeclareTxn::V1(tx) => &mut tx.signature,
                BroadcastedDeclareTxn::V2(tx) => &mut tx.signature,
                BroadcastedDeclareTxn::V3(tx) => &mut tx.signature,
                _ => unreachable!("the declare tx is not query only"),
            };
            *tx_signature = vec![signature.r, signature.s].into();

            self.tx_validator.submit_declare_transaction(tx).await
        }

        pub async fn sign_and_add_deploy_account_tx(
            &self,
            mut tx: BroadcastedDeployAccountTxn,
            contract: &DevnetPredeployedContract,
        ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
            let api_tx = BroadcastedTxn::DeployAccount(tx.clone())
                .into_validated_tx(
                    self.backend.chain_config().chain_id.to_felt(),
                    self.backend.chain_config().latest_protocol_version,
                    TxTimestamp::now(),
                )
                .unwrap();
            let signature = contract.secret.sign(&api_tx.tx_hash).unwrap();

            let tx_signature = match &mut tx {
                BroadcastedDeployAccountTxn::V1(tx) => &mut tx.signature,
                BroadcastedDeployAccountTxn::V3(tx) => &mut tx.signature,
                _ => unreachable!("the deploy account tx is not query only"),
            };
            *tx_signature = vec![signature.r, signature.s].into();

            self.tx_validator.submit_deploy_account_transaction(tx).await
        }

        /// (STRK in FRI, ETH in WEI)
        pub fn get_bal_strk_eth(&self, contract_address: Felt) -> (u128, u128) {
            get_fee_tokens_balance(&self.backend.view_on_latest(), contract_address).unwrap().as_u128_fri_wei().unwrap()
        }
    }

    async fn test_chain() -> DevnetForTesting {
        test_chain_with_chain_config(ChainConfig::madara_devnet()).await
    }
    async fn test_chain_with_block_time(block_time: Duration) -> DevnetForTesting {
        let mut chain_config = ChainConfig::madara_devnet();
        chain_config.block_time = block_time;
        test_chain_with_chain_config(chain_config).await
    }
    async fn test_chain_with_chain_config(chain_config: ChainConfig) -> DevnetForTesting {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mut g = ChainGenesisDescription::base_config().unwrap();
        let contracts = g.add_devnet_contracts(10).unwrap();

        let chain_config = Arc::new(chain_config);
        let backend = MadaraBackend::open_for_testing(chain_config.clone());
        g.build_and_store(&backend).await.unwrap();
        tracing::debug!(
            "block imported {:?}",
            backend.block_view_on_last_confirmed().unwrap().get_block_info().unwrap()
        );

        let mut l1_data_provider = MockL1DataProvider::new();
        l1_data_provider.expect_get_gas_prices().return_const(GasPrices {
            eth_l1_gas_price: 128,
            strk_l1_gas_price: 128,
            eth_l1_data_gas_price: 128,
            strk_l1_data_gas_price: 128,
        });
        let l1_data_provider = Arc::new(l1_data_provider) as Arc<dyn L1DataProvider>;
        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
        let metrics = BlockProductionMetrics::register();

        let block_production = BlockProductionTask::new(
            Arc::clone(&backend),
            Arc::clone(&mempool),
            Arc::new(metrics),
            Arc::clone(&l1_data_provider),
            Arc::new(mc_settlement_client::L1SyncDisabledClient) as _,
        );

        let tx_validator = Arc::new(TransactionValidator::new(
            Arc::clone(&mempool) as _,
            Arc::clone(&backend),
            TransactionValidatorConfig::default(),
        ));

        DevnetForTesting { backend, contracts, block_production: Some(block_production), tx_validator }
    }

    #[rstest]
    #[case(m_cairo_test_contracts::TEST_CONTRACT_SIERRA)]
    #[tokio::test]
    async fn test_declare(#[case] contract: &[u8]) {
        let mut chain = test_chain().await;
        tracing::info!("{}", chain.contracts);

        let sender_address = &chain.contracts.0[0];

        let sierra_class: SierraClass = serde_json::from_slice(contract).unwrap();
        let flattened_class: FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();

        let (compiled_contract_class_hash, _compiled_class) = flattened_class.compile_to_casm().unwrap();

        let declare_txn: BroadcastedDeclareTxn = BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
            sender_address: sender_address.address,
            compiled_class_hash: compiled_contract_class_hash,
            signature: vec![].into(),
            nonce: Felt::ZERO,
            contract_class: flattened_class.into(),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 220000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let res = chain.sign_and_add_declare_tx(declare_txn, sender_address).await.unwrap();

        let calculated_class_hash = sierra_class.class_hash().unwrap();

        assert_eq!(res.class_hash, calculated_class_hash);

        let mut block_production = chain.block_production.take().unwrap();
        let mut notifications = block_production.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(async move { block_production.run(ServiceContext::new_for_testing()).await.unwrap() });
        for _ in 0..10 {
            assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);
            if !chain.backend.block_view_on_preconfirmed_or_fake().get_block_info().tx_hashes.is_empty() {
                break;
            }
        }
        let pending_view = chain.backend.block_view_on_preconfirmed_or_fake();
        assert_eq!(pending_view.get_block_info().tx_hashes.len(), 1);
        tracing::debug!("receipt: {:?}", pending_view.get_transaction(0).unwrap().receipt);

        let class_info = chain.backend.view_on_latest().get_class_info(&calculated_class_hash).unwrap().unwrap();

        assert_matches!(
            class_info,
            ClassInfo::Sierra(info) if info.compiled_class_hash == compiled_contract_class_hash
        );

        let TransactionReceipt::Declare(receipt) = pending_view.get_transaction(0).unwrap().receipt else {
            unreachable!()
        };

        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);
    }

    #[rstest]
    #[case::should_fail_no_fund(false, false, Duration::from_secs(500000), false)]
    #[case::should_work_all_in_pending_block(true, false, Duration::from_secs(500000), true)]
    #[case::should_work_across_block_boundary(true, true, Duration::from_millis(300), true)]
    // FIXME: flaky
    // #[case::should_work_across_block_boundary(true, true, None, Duration::from_secs(1), true)]
    #[ignore = "should_work_across_block_boundary"]
    #[tokio::test]
    async fn test_account_deploy(
        #[case] transfer_fees: bool,
        #[case] wait_block_time: bool,
        #[case] block_time: Duration,
        #[case] should_work: bool,
    ) {
        let mut chain = test_chain_with_block_time(block_time).await;

        let mut block_production = chain.block_production.take().unwrap();
        let mut notifications = block_production.subscribe_state_notifications();
        let mut _task =
            AbortOnDrop::spawn(async move { block_production.run(ServiceContext::new_for_testing()).await.unwrap() });

        let key = SigningKey::from_random();
        tracing::debug!("Secret Key : {:?}", key.secret_scalar());

        let pubkey = key.verifying_key();
        tracing::debug!("Public Key : {:?}", pubkey.scalar());

        // using the class hash of the first account as the account class hash
        let account_class_hash = chain.contracts.0[0].class_hash;
        let calculated_address =
            calculate_contract_address(Felt::ZERO, account_class_hash, &[pubkey.scalar()], Felt::ZERO);
        tracing::debug!("Calculated Address : {:?}", calculated_address);

        if transfer_fees {
            // =====================================================================================
            // Transferring the funds from pre deployed account into the calculated address
            let contract_0 = &chain.contracts.0[0];

            let transfer_txn = chain
                .sign_and_add_invoke_tx(
                    BroadcastedInvokeTxn::V3(InvokeTxnV3 {
                        sender_address: contract_0.address,
                        calldata: Multicall::default()
                            .with(Call {
                                to: ERC20_STRK_CONTRACT_ADDRESS,
                                selector: Selector::from("transfer"),
                                calldata: vec![calculated_address, (9_999u128 * STRK_FRI_DECIMALS).into(), Felt::ZERO],
                            })
                            .flatten()
                            .collect::<Vec<_>>()
                            .into(),
                        signature: vec![].into(), // Signature is filled in by `sign_and_add_invoke_tx`.
                        nonce: Felt::ZERO,
                        resource_bounds: ResourceBoundsMapping {
                            l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                            l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                        },
                        tip: 0,
                        paymaster_data: vec![],
                        account_deployment_data: vec![],
                        nonce_data_availability_mode: DaMode::L1,
                        fee_data_availability_mode: DaMode::L1,
                    }),
                    contract_0,
                )
                .await
                .unwrap();
            tracing::debug!("tx hash: {:#x}", transfer_txn.transaction_hash);
            let notif = if wait_block_time {
                BlockProductionStateNotification::ClosedBlock
            } else {
                BlockProductionStateNotification::UpdatedPendingBlock
            };

            for _ in 0..10 {
                assert_eq!(notifications.recv().await.unwrap(), notif);
                if !chain.backend.block_view_on_preconfirmed_or_fake().get_block_info().tx_hashes.is_empty() {
                    break;
                }
            }
        }

        // =====================================================================================

        let account_balance = get_fee_tokens_balance(&chain.backend.view_on_latest(), calculated_address).unwrap();
        let account = DevnetPredeployedContract {
            secret: key,
            pubkey: pubkey.scalar(),
            balance: account_balance,
            address: calculated_address,
            class_hash: account_class_hash,
        };

        let deploy_account_txn = BroadcastedDeployAccountTxn::V3(DeployAccountTxnV3 {
            signature: vec![].into(),
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
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let res = chain.sign_and_add_deploy_account_tx(deploy_account_txn, &account).await;

        if !should_work {
            assert_matches!(
                res,
                Err(SubmitTransactionError::Rejected(RejectedTransactionError {
                    kind: RejectedTransactionErrorKind::ValidateFailure,
                    ..
                }))
            );
            assert!(format!("{:#}", res.unwrap_err()).contains("exceed balance"));
            return;
        }

        let res = res.unwrap();

        let notif = if wait_block_time {
            BlockProductionStateNotification::ClosedBlock
        } else {
            BlockProductionStateNotification::UpdatedPendingBlock
        };

        for _ in 0..10 {
            assert_eq!(notifications.recv().await.unwrap(), notif);
            if !chain.backend.block_view_on_preconfirmed_or_fake().get_block_info().tx_hashes.is_empty() {
                break;
            }
        }

        assert_eq!(res.contract_address, account.address);

        let res = chain.backend.view_on_latest().get_transaction_by_hash(&res.transaction_hash).unwrap();
        let (_, tx) = res.unwrap();

        let TransactionReceipt::DeployAccount(receipt) = tx.receipt else { unreachable!() };

        assert_eq!(receipt.execution_result, ExecutionResult::Succeeded);
    }

    // TODO: add eth transfer
    #[rstest]
    #[case(24235u128, false)]
    #[case(9_999u128 * STRK_FRI_DECIMALS, false)]
    #[case(10_001u128 * STRK_FRI_DECIMALS, true)]
    #[tokio::test]
    async fn test_basic_transfer(#[case] transfer_amount: u128, #[case] expect_reverted: bool) {
        let mut chain = test_chain().await;
        tracing::info!("{}", chain.contracts);

        let sequencer_address = chain.backend.chain_config().sequencer_address.to_felt();
        let contract_0 = &chain.contracts.0[0];
        let contract_1 = &chain.contracts.0[1];

        assert_eq!(chain.get_bal_strk_eth(sequencer_address), (0, 0));
        assert_eq!(chain.get_bal_strk_eth(contract_0.address), (10_000 * STRK_FRI_DECIMALS, 10_000 * ETH_WEI_DECIMALS));
        assert_eq!(chain.get_bal_strk_eth(contract_1.address), (10_000 * STRK_FRI_DECIMALS, 10_000 * ETH_WEI_DECIMALS));

        let result = chain
            .sign_and_add_invoke_tx(
                BroadcastedInvokeTxn::V3(InvokeTxnV3 {
                    sender_address: contract_0.address,
                    calldata: Multicall::default()
                        .with(Call {
                            to: ERC20_STRK_CONTRACT_ADDRESS,
                            selector: Selector::from("transfer"),
                            calldata: vec![contract_1.address, transfer_amount.into(), Felt::ZERO],
                        })
                        .flatten()
                        .collect::<Vec<_>>()
                        .into(),
                    signature: vec![].into(), // Signature is filled in by `sign_and_add_invoke_tx`.
                    nonce: Felt::ZERO,
                    resource_bounds: ResourceBoundsMapping {
                        l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                        l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                    },
                    tip: 0,
                    paymaster_data: vec![],
                    account_deployment_data: vec![],
                    nonce_data_availability_mode: DaMode::L1,
                    fee_data_availability_mode: DaMode::L1,
                }),
                contract_0,
            )
            .await
            .unwrap();

        tracing::info!("tx hash: {:#x}", result.transaction_hash);

        let mut block_production = chain.block_production.take().unwrap();
        let mut notifications = block_production.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(async move { block_production.run(ServiceContext::new_for_testing()).await.unwrap() });

        for _ in 0..10 {
            assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);
            if !chain.backend.block_view_on_preconfirmed_or_fake().get_block_info().tx_hashes.is_empty() {
                break;
            }
        }

        let block_txs = chain.backend.block_view_on_preconfirmed_or_fake().get_block_transactions(..);

        assert_eq!(block_txs.len(), 1);
        tracing::info!("receipt: {:?}", block_txs[0].receipt);

        let TransactionReceipt::Invoke(receipt) = block_txs[0].receipt.clone() else { unreachable!() };
        let fees_fri = block_txs[0].receipt.actual_fee().amount;

        if !expect_reverted {
            assert_eq!(
                receipt,
                InvokeTransactionReceipt {
                    transaction_hash: result.transaction_hash,
                    messages_sent: vec![],
                    events: vec![
                        Event {
                            from_address: ERC20_STRK_CONTRACT_ADDRESS,
                            keys: vec![
                                // Transfer
                                Felt::from_hex_unchecked(
                                    "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                                ),
                                // From
                                contract_0.address,
                                // To
                                contract_1.address,
                            ],
                            // U256 of the amount
                            data: vec![transfer_amount.into(), Felt::ZERO],
                        },
                        Event {
                            from_address: ERC20_STRK_CONTRACT_ADDRESS,
                            keys: vec![
                                Felt::from_hex_unchecked(
                                    "0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9"
                                ),
                                contract_0.address,
                                sequencer_address,
                            ],
                            // This is the fees transfer to the sequencer.
                            data: vec![fees_fri, Felt::ZERO],
                        },
                    ],
                    // TODO: resources and fees are not tested because they consistent accross runs, we have to figure out why
                    execution_resources: receipt.execution_resources.clone(),
                    actual_fee: FeePayment { amount: fees_fri, unit: PriceUnit::Fri },
                    execution_result: receipt.execution_result.clone(), // matched below
                }
            );
        }

        match expect_reverted {
            false => {
                assert_eq!(&receipt.execution_result, &ExecutionResult::Succeeded);

                let fees_fri = block_txs[0].receipt.actual_fee().amount.try_into().unwrap();
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

                let fees_fri = block_txs[0].receipt.actual_fee().amount.try_into().unwrap();
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
