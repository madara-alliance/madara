use blockifier::abi::abi_utils::get_storage_var_address;
use dc_block_import::{UnverifiedFullBlock, UnverifiedHeader};
use dp_block::header::GasPrices;
use dp_chain_config::ChainConfig;
use dp_convert::ToFelt;
use dp_state_update::{ContractStorageDiffItem, StateDiff, StorageEntry};
use starknet_api::{core::ContractAddress, state::StorageKey};
use starknet_signers::SigningKey;
use starknet_types_core::felt::Felt;
use std::{collections::HashMap, fmt, time::SystemTime};

mod balances;
mod classes;
mod contracts;
mod entrypoint;

pub use balances::*;
pub use classes::*;
pub use contracts::*;
pub use entrypoint::*;

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
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_UniversalDeployer.contract_class.json");
const UDC_CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x07b3e05f48f0c69e4a65ce5e076a66271a527aff2c34ce1083ec6e1526997a69");
const UDC_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf");

const ERC20_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_ERC20.contract_class.json");
const ERC20_CLASS_HASH: Felt =
    Felt::from_hex_unchecked("0x04ad3c1dc8413453db314497945b6903e1c766495a1e60492d44da9c2a986e4b");
const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_CLASS_DEFINITION: &[u8] =
    include_bytes!("../../../../../cairo/target/dev/madara_contracts_AccountUpgradeable.contract_class.json");
const ACCOUNT_CLASS_HASH: Felt = Felt::from_hex_unchecked("0xFFFFFFAFAFAFAFAFAFA9b9b9b");

/// High level description of the genesis block.
#[derive(Clone, Debug, Default)]
pub struct ChainGenesisDescription {
    pub initial_balances: InitialBalances,
    pub declared_classes: InitiallyDeclaredClasses,
    pub deployed_contracts: InitiallyDeployedContracts,
    /// This is filled in with the initial_balances too when building.
    pub initial_storage: StorageDiffs,
}

pub struct DevnetPredeployedContract {
    pub address: Felt,
    pub secret: SigningKey,
    pub pubkey: Felt,
    pub balance: InitialBalance,
}

pub struct DevnetKeys(Vec<DevnetPredeployedContract>);

impl fmt::Display for DevnetKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "==== DEVNET PREDEPLOYED CONTRACTS ====")?;
        writeln!(f)?;
        for (i, keypair) in self.0.iter().enumerate() {
            writeln!(
                f,
                "#{} - Address: {:#x}, Private key: {:#x}",
                i,
                keypair.address,
                keypair.secret.secret_scalar()
            )?;
            // writeln!(f, "Balance: 10000 ETH, 10000 STRK")?;
            // writeln!(f, "")?;
        }
        Ok(())
    }
}

impl ChainGenesisDescription {
    pub fn base_config() -> Self {
        Self {
            initial_balances: InitialBalances::default(),
            declared_classes: InitiallyDeclaredClasses::default()
                .with(InitiallyDeclaredClass::new_sierra(UDC_CLASS_HASH, Felt::ONE, UDC_CLASS_DEFINITION))
                .with(InitiallyDeclaredClass::new_sierra(ERC20_CLASS_HASH, Felt::ONE, ERC20_CLASS_DEFINITION)),
            deployed_contracts: InitiallyDeployedContracts::default()
                .with(UDC_CONTRACT_ADDRESS, UDC_CLASS_HASH)
                .with(ERC20_ETH_CONTRACT_ADDRESS, ERC20_CLASS_HASH)
                .with(ERC20_STRK_CONTRACT_ADDRESS, ERC20_CLASS_HASH),
            initial_storage: StorageDiffs::default(),
        }
    }

    pub fn add_devnet_contracts(&mut self, n_addr: u64) -> DevnetKeys {
        self.declared_classes.insert(InitiallyDeclaredClass::new_sierra(
            ACCOUNT_CLASS_HASH,
            Felt::ONE,
            ACCOUNT_CLASS_DEFINITION,
        ));

        fn get_contract_pubkey_storage_address() -> StorageKey {
            get_storage_var_address("Account_public_key", &[])
        }

        DevnetKeys(
            (0..n_addr)
                .map(|_| {
                    let key = SigningKey::from_random();
                    let pubkey = key.verifying_key();

                    let address = pubkey.scalar().double(); // let's just lie and make it up here for now

                    let balance = InitialBalance {
                        fri: (10_000 * ETH_WEI_DECIMALS).into(),
                        wei: (10_000 * STRK_FRI_DECIMALS).into(),
                    };

                    self.deployed_contracts.insert(address, ACCOUNT_CLASS_HASH);
                    self.initial_balances.insert(ContractAddress::try_from(address).unwrap(), balance.clone());
                    self.initial_storage
                        .contract_mut(address.try_into().unwrap())
                        .insert(get_contract_pubkey_storage_address(), pubkey.scalar());

                    DevnetPredeployedContract { secret: key, pubkey: pubkey.scalar(), balance, address }
                })
                .collect(),
        )
    }

    pub fn build(mut self, chain_config: &ChainConfig) -> anyhow::Result<UnverifiedFullBlock> {
        self.initial_balances.to_storage_diffs(chain_config, &mut self.initial_storage);

        Ok(UnverifiedFullBlock {
            header: UnverifiedHeader {
                parent_block_hash: None,
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
                l1_da_mode: dp_block::header::L1DataAvailabilityMode::Blob,
            },
            state_diff: StateDiff {
                storage_diffs: self.initial_storage.as_state_diff(),
                deprecated_declared_classes: self.declared_classes.as_legacy_state_diff(),
                declared_classes: self.declared_classes.as_state_diff(),
                deployed_contracts: self.deployed_contracts.as_state_diff(),
                replaced_classes: vec![],
                nonces: vec![],
            },
            declared_classes: self.declared_classes.into_loaded_classes()?,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l1::MockL1DataProvider;
    use crate::{block_production::BlockProductionTask, Mempool, MempoolProvider};
    use crate::{transaction_hash, L1DataProvider};
    use blockifier::abi::abi_utils::get_fee_token_var_address;
    use blockifier::abi::sierra_types::next_storage_key;
    use dc_block_import::{BlockImporter, Validation};
    use dc_db::DeoxysBackend;
    use dp_block::header::L1DataAvailabilityMode;
    use dp_block::{BlockId, BlockTag};
    use dp_convert::felt_to_u128;
    use dp_receipt::{Event, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt};
    use dp_transactions::broadcasted_to_blockifier;
    use rstest::{fixture, rstest};
    use starknet_core::types::{
        BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
        BroadcastedInvokeTransactionV3, BroadcastedTransaction, DeclareTransactionResult,
        DeployAccountTransactionResult, InvokeTransactionResult, ResourceBounds, ResourceBoundsMapping,
    };

    use std::sync::Arc;

    struct PreparedDevnet {
        backend: Arc<DeoxysBackend>,
        contracts: DevnetKeys,
        block_production: BlockProductionTask,
        mempool: Arc<Mempool>,
    }

    impl PreparedDevnet {
        pub fn sign_and_add_invoke_tx(
            &self,
            mut tx: BroadcastedInvokeTransaction,
            contract: &DevnetPredeployedContract,
        ) -> InvokeTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::Invoke(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
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

        #[allow(unused)]
        pub fn sign_and_add_declare_tx(
            &self,
            mut tx: BroadcastedDeclareTransaction,
            contract: &DevnetPredeployedContract,
        ) -> DeclareTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::Declare(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
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

        #[allow(unused)]
        pub fn sign_and_add_deploy_account_tx(
            &self,
            mut tx: BroadcastedDeployAccountTransaction,
            contract: &DevnetPredeployedContract,
        ) -> DeployAccountTransactionResult {
            let (blockifier_tx, _classes) = broadcasted_to_blockifier(
                BroadcastedTransaction::DeployAccount(tx.clone()),
                self.backend.chain_config().chain_id.to_felt(),
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

        /// Returns an `u128`. This is for tests only as an ERC20 contract may have a higher balance than an u128.
        pub fn get_bal_contract(&self, contract_address: Felt, fee_token_address: Felt) -> u128 {
            let low_key = get_fee_token_var_address(contract_address.try_into().unwrap());
            let high_key = next_storage_key(&low_key).unwrap();
            let low = self
                .backend
                .get_contract_storage_at(&BlockId::Tag(BlockTag::Pending), &fee_token_address, &low_key)
                .unwrap()
                .unwrap_or(Felt::ZERO);
            let high = self
                .backend
                .get_contract_storage_at(&BlockId::Tag(BlockTag::Pending), &fee_token_address, &high_key)
                .unwrap()
                .unwrap_or(Felt::ZERO);
            log::debug!("get_fee_token_balance contract_address={contract_address:#x} fee_token_address={fee_token_address:#x} low_key={low_key:?}, got {low:#x} {high:#x}");

            assert_eq!(high, Felt::ZERO); // for now we never use high let's keep it out of the api
                                          // (blockifier does not even support it fully I believe, as the total supply of STRK/ETH would not reach the high bits.)

            felt_to_u128(&low).unwrap()
        }

        /// (STRK in FRI, ETH in WEI)
        pub fn get_bal_strk_eth(&self, contract_address: Felt) -> (u128, u128) {
            (
                self.get_bal_contract(contract_address, ERC20_STRK_CONTRACT_ADDRESS),
                self.get_bal_contract(contract_address, ERC20_ETH_CONTRACT_ADDRESS),
            )
        }
    }

    #[fixture]
    fn chain() -> PreparedDevnet {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut g = ChainGenesisDescription::base_config();
        let contracts = g.add_devnet_contracts(10);

        let chain_config = Arc::new(ChainConfig::test_config());
        let block = g.build(&chain_config).unwrap();
        let backend = DeoxysBackend::open_for_testing(Arc::clone(&chain_config));
        let importer = BlockImporter::new(Arc::clone(&backend));

        println!("{:?}", block.state_diff);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(importer.add_block(
                block,
                Validation {
                    trust_transaction_hashes: false,
                    trust_global_tries: false,
                    chain_id: chain_config.chain_id.clone(),
                },
            ))
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
        let block_production =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), Arc::clone(&l1_data_provider))
                .unwrap();

        PreparedDevnet { backend, contracts, block_production, mempool }
    }

    // TODO: add eth transfer
    #[rstest]
    #[case(24235u128, false)]
    #[case(9_999u128 * STRK_FRI_DECIMALS, false)]
    #[case(10_001u128 * STRK_FRI_DECIMALS, true)]
    fn test_basic_transfer(mut chain: PreparedDevnet, #[case] transfer_amount: u128, #[case] expect_reverted: bool) {
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

        chain.block_production.current_pending_tick = 1;
        chain.block_production.on_pending_time_tick().unwrap();

        let block = chain.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap();

        assert_eq!(block.inner.transactions.len(), 1);
        assert_eq!(block.inner.receipts.len(), 1);
        log::info!("receipt: {:?}", block.inner.receipts[0]);

        let TransactionReceipt::Invoke(receipt) = block.inner.receipts[0].clone() else { unreachable!() };
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
